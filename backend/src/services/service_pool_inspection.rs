//! Read-only owner-scoped pool management inspection. Never decrypts credentials.
use super::{proxy_service, service_pool_health_service as health, service_pool_service};
use crate::{
    crypto::aes::EncryptionKeys,
    errors::{AppError, AppResult},
    models::{
        service_pool::{PoolMemberContract, PoolStrategy, ServicePool},
        user_service::UserService,
    },
};
use futures::TryStreamExt;
use mongodb::bson::doc;
use std::collections::HashSet;

pub struct CandidateInspection {
    pub user_service_id: String,
    pub name: String,
    pub slug: String,
    pub is_active: bool,
    pub eligible: bool,
    pub reason: Option<String>,
    pub credential_binding: String,
    pub protocol: Option<crate::models::downstream_service::InferenceWireProtocol>,
    pub catalog_service_id: Option<String>,
    pub requires_compatibility_declaration: bool,
    pub cooldown_until: Option<chrono::DateTime<chrono::Utc>>,
    pub consecutive_failures: i64,
    pub last_status: Option<i32>,
}

pub struct InspectionPage {
    pub candidates: Vec<CandidateInspection>,
    pub next_cursor: Option<String>,
}

#[derive(Default)]
pub struct InspectionQuery<'a> {
    pub after: Option<&'a str>,
    pub search: Option<&'a str>,
    pub limit: u32,
    pub members_only: bool,
    pub inventory_only: bool,
    pub selected_only: bool,
    pub peer_ids: Option<&'a [String]>,
    pub declared_peer_ids: Option<&'a str>,
}

#[allow(clippy::too_many_arguments)]
pub async fn inspect(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    actor: &str,
    agent_key: Option<&str>,
    owner: &str,
    pool: Option<&ServicePool>,
    strategy: PoolStrategy,
    contract: PoolMemberContract,
    method: &str,
    path: &str,
    allowed_services: Option<&HashSet<String>>,
    allowed_nodes: Option<&HashSet<String>>,
    query: InspectionQuery<'_>,
) -> AppResult<InspectionPage> {
    let strategy = if query.members_only {
        pool.map_or(strategy, |pool| pool.strategy)
    } else {
        strategy
    };
    let contract = if query.members_only {
        pool.map_or(contract, |pool| pool.member_contract)
    } else {
        contract
    };
    let saved_operation = !query.inventory_only
        && (query.members_only
            || (query.peer_ids.is_none()
                && query.declared_peer_ids.is_none()
                && pool.is_some_and(|p| p.strategy == strategy && p.member_contract == contract)));
    let priority = strategy == PoolStrategy::Priority;
    let contract = if priority {
        contract
    } else {
        PoolMemberContract::SameApi
    };
    let declarations: HashSet<&str> = query
        .declared_peer_ids
        .filter(|_| !query.members_only)
        .unwrap_or("")
        .split(',')
        .filter(|s| !s.is_empty())
        .collect();
    let mut filter = doc! { "user_id": owner };
    if let Some(allowed) = allowed_services {
        filter.insert(
            "_id",
            doc! {"$in":allowed.iter().cloned().collect::<Vec<_>>()},
        );
    }
    let offset = query
        .after
        .filter(|_| !query.members_only && !query.selected_only)
        .map(str::parse::<u64>)
        .transpose()
        .map_err(|_| crate::errors::AppError::BadRequest("Invalid candidate cursor".into()))?
        .unwrap_or(0);
    if offset > 1_000_000 {
        return Err(crate::errors::AppError::BadRequest(
            "Candidate cursor exceeds supported inventory size; narrow the search".into(),
        ));
    }
    if query.members_only {
        filter.insert("_id", doc! { "$in": pool.map(|p| p.members.iter().filter(|m| allowed_services.is_none_or(|a| a.contains(&m.user_service_id))).map(|m| m.user_service_id.clone()).collect::<Vec<_>>()).unwrap_or_default() });
    } else if query.selected_only {
        filter.insert("_id", doc! {"$in": query.peer_ids.unwrap_or_default().iter().filter(|id| allowed_services.is_none_or(|allowed| allowed.contains(*id))).cloned().collect::<Vec<_>>()});
    }
    let limit = if query.members_only || query.selected_only {
        50
    } else {
        query.limit.clamp(1, 100) as usize
    };
    let search = query
        .search
        .filter(|search| !query.members_only && !query.selected_only && !search.is_empty());
    let mut services: Vec<UserService> = if let Some(search) = search {
        search_services(db, filter, search, offset, limit + 1).await?
    } else {
        crate::services::service_history::collection(db, "user_services")
            .find(filter)
            .sort(doc! {"_id":1})
            .skip(offset)
            .limit((limit + 1) as i64)
            .await?
            .try_collect()
            .await?
    };
    let has_more = services.len() > limit;
    services.truncate(limit);
    let next_cursor = has_more.then(|| (offset + limit as u64).to_string());
    let selected = if let Some(peers) = query.peer_ids.filter(|_| !query.members_only) {
        let members: Vec<_> = peers
            .iter()
            .filter(|id| allowed_services.is_none_or(|a| a.contains(*id)))
            .map(|id| crate::models::service_pool::ServicePoolMember {
                user_service_id: id.clone(),
                weight: 1,
                enabled: true,
                priority: 0,
                model: None,
                same_api_compatible: declarations.contains(id.as_str()),
                health_reset_generation: 0,
            })
            .collect();
        super::service_pool_contract::load_metadata(db, owner, &members, true).await?
    } else if let Some(pool) = pool {
        super::service_pool_contract::load_metadata(db, owner, &pool.members, true).await?
    } else {
        Vec::new()
    };
    let mut result = Vec::new();
    for service in services {
        // Restricted callers cannot enumerate excluded service identities.
        if allowed_services.is_some_and(|allowed| !allowed.contains(&service.id)) {
            continue;
        }
        let member = pool.and_then(|pool| {
            pool.members
                .iter()
                .find(|m| m.user_service_id == service.id)
        });
        let mut row = CandidateInspection {
            user_service_id: service.id.clone(),
            name: service.slug.clone(),
            slug: service.slug.clone(),
            is_active: service.is_active,
            eligible: true,
            reason: None,
            credential_binding: service
                .credential_binding
                .clone()
                .unwrap_or_else(|| "user".into()),
            protocol: None,
            catalog_service_id: service.catalog_service_id.clone(),
            requires_compatibility_declaration: service.catalog_service_id.is_none(),
            cooldown_until: None,
            consecutive_failures: 0,
            last_status: None,
        };
        if !service.is_active {
            row.reason = Some("inactive".into());
        } else if service.service_type == "ssh" {
            row.reason = Some("unsupported_transport".into());
        } else {
            let resolution = proxy_service::read_proxy_authority_snapshot_by_user_service_id(
                db,
                keys,
                actor,
                &service.id,
                None,
            )
            .await;
            match resolution {
                Ok(Some(resolution)) => {
                    row.name = resolution.target.service.name.clone();
                    row.credential_binding = if resolution.master_credential {
                        "platform"
                    } else if resolution.target.auth_method == "none" {
                        "none"
                    } else {
                        "user"
                    }
                    .into();
                    row.protocol = resolution
                        .target
                        .service
                        .inference
                        .as_ref()
                        .map(|i| i.wire_protocol);
                    if resolution.node_id.as_ref().is_some_and(|id| {
                        allowed_nodes.is_some_and(|allowed| !allowed.contains(id))
                    }) {
                        continue;
                    }
                    if priority
                        && let Some(reason) = service_pool_service::node_eligibility_reason(
                            db,
                            owner,
                            &resolution,
                            allowed_nodes,
                        )
                        .await?
                    {
                        row.reason = Some(reason.into());
                    }
                    let native_path = if contract == PoolMemberContract::AiChat {
                        match super::pool_ai_service::prepare(
                            &resolution.target.service,
                            resolution.catalog_service_slug.as_deref(),
                            member
                                .and_then(|m| m.model.as_deref())
                                .unwrap_or("candidate-model"),
                            &http::Method::from_bytes(if query.inventory_only {
                                b"POST"
                            } else {
                                method.as_bytes()
                            })
                            .map_err(|_| {
                                crate::errors::AppError::BadRequest("Invalid method".into())
                            })?,
                            if query.inventory_only {
                                "chat/completions"
                            } else {
                                path
                            },
                            br#"{"messages":[{"role":"user","content":""}]}"#,
                        ) {
                            Ok(prepared) => prepared.path,
                            Err(_) => {
                                row.reason = Some("inference_protocol_required".into());
                                String::new()
                            }
                        }
                    } else {
                        path.to_owned()
                    };
                    if !query.inventory_only && row.reason.is_none() {
                        let canonical =
                            super::proxy_authorization::CanonicalPath::from_rest_decoded(
                                &native_path,
                            )?;
                        if super::destination_routing::select_target(
                            &resolution.target.service,
                            method,
                            &canonical,
                        )
                        .is_err()
                        {
                            row.reason = Some("operation_unsupported".into());
                        }
                    }
                    let credential_override = if let Some(agent) = agent_key {
                        match proxy_service::read_agent_credential_override_identity(
                            db,
                            actor,
                            agent,
                            &service.id,
                            &resolution.target,
                        )
                        .await
                        {
                            Ok(identity) => identity,
                            Err(AppError::CredentialUnavailable(_)) => {
                                row.reason = Some("credential_unavailable".into());
                                None
                            }
                            Err(error) => return Err(error),
                        }
                    } else {
                        None
                    };
                    if saved_operation
                        && priority
                        && row.reason.is_none()
                        && let Some(pool) = pool
                    {
                        let scope = health::scope_from_resolution(
                            db,
                            &pool.id,
                            owner,
                            pool.config_revision,
                            &resolution,
                            member.and_then(|m| m.model.clone()),
                            credential_override.as_ref(),
                            method,
                            &native_path,
                        )
                        .await;
                        match scope {
                            Ok(scope) => {
                                if let Some(current) = health::load_for_scope(db, &scope).await? {
                                    row.cooldown_until = current
                                        .cooldown_until
                                        .filter(|until| *until > chrono::Utc::now());
                                    row.consecutive_failures = current.consecutive_failures as i64;
                                    row.last_status = current.last_status;
                                }
                            }
                            Err(error) if service_pool_service::member_unavailable(&error) => {
                                row.reason = Some("operation_unsupported".into())
                            }
                            Err(error) => return Err(error),
                        }
                    }
                }
                Ok(None) => row.reason = Some("unavailable".into()),
                Err(AppError::CredentialUnavailable(_)) => {
                    row.reason = Some("credential_unavailable".into())
                }
                Err(error) if service_pool_service::member_unavailable(&error) => {
                    row.reason = Some("unavailable".into())
                }
                Err(error) => return Err(error),
            }
        }
        if (query.members_only || saved_operation)
            && row.reason.is_none()
            && member.is_some_and(|m| !m.enabled)
        {
            row.reason = Some("disabled".into());
        }
        if row.reason.is_none() && row.cooldown_until.is_some() {
            row.reason = Some("cooldown".into());
        }
        row.eligible = row.reason.is_none();
        result.push(row);
    }
    if query.members_only
        && let Some(pool) = pool
    {
        for member in &pool.members {
            if result
                .iter()
                .any(|row| row.user_service_id == member.user_service_id)
                || allowed_services
                    .is_some_and(|allowed| !allowed.contains(&member.user_service_id))
            {
                continue;
            }
            if db
                .collection::<UserService>("user_services")
                .find_one(doc! {"_id":&member.user_service_id,"user_id":owner})
                .await?
                .is_some()
            {
                continue;
            }
            result.push(CandidateInspection {
                user_service_id: member.user_service_id.clone(),
                name: "Unavailable connection".into(),
                slug: "Unavailable member".into(),
                is_active: false,
                eligible: false,
                reason: Some("unavailable".into()),
                credential_binding: "unavailable".into(),
                protocol: None,
                catalog_service_id: None,
                requires_compatibility_declaration: false,
                cooldown_until: None,
                consecutive_failures: 0,
                last_status: None,
            });
        }
    }
    let metadata: Vec<_> = result
        .iter()
        .map(|row| super::service_pool_contract::ContractMember {
            catalog_id: row.catalog_service_id.clone(),
            protocol: row.protocol,
            declared: declarations.contains(row.user_service_id.as_str())
                || ((query.members_only || query.peer_ids.is_none())
                    && pool
                        .and_then(|pool| {
                            pool.members
                                .iter()
                                .find(|m| m.user_service_id == row.user_service_id)
                        })
                        .is_some_and(|m| m.same_api_compatible)),
        })
        .collect();
    for (row, member) in result.iter_mut().zip(&metadata) {
        let mut peers = selected.clone();
        peers.push(member.clone());
        row.requires_compatibility_declaration = priority
            && contract == PoolMemberContract::SameApi
            && super::service_pool_contract::declaration_required(member, &peers);
        if priority
            && row.reason.is_none()
            && contract == PoolMemberContract::SameApi
            && super::service_pool_contract::validate(contract, &peers).is_err()
        {
            row.eligible = false;
            let mut declared_peers = peers.clone();
            for peer in &mut declared_peers {
                peer.declared = true;
            }
            row.reason = Some(
                if super::service_pool_contract::validate(contract, &declared_peers).is_err() {
                    "incompatible_protocol"
                } else {
                    "compatibility_declaration_required"
                }
                .into(),
            );
        }
    }
    Ok(InspectionPage {
        candidates: result,
        next_cursor,
    })
}

/// Search before pagination, projecting only the referenced endpoint label.
/// Both personal and platform resolution use this label as the displayed name.
async fn search_services(
    db: &mongodb::Database,
    filter: mongodb::bson::Document,
    search: &str,
    offset: u64,
    limit: usize,
) -> AppResult<Vec<UserService>> {
    let pattern = doc! {"$regex":regex::escape(search),"$options":"i"};
    Ok(db
        .collection::<UserService>("user_services")
        .aggregate([
            doc! {"$match":filter},
            doc! {"$sort":{"_id":1}},
            doc! {"$lookup":{
                "from":"user_endpoints","localField":"endpoint_id","foreignField":"_id",
                "pipeline":[{"$project":{"_id":0,"label":1}}],"as":"search_endpoint",
            }},
            doc! {"$match":{"$or":[{"slug":&pattern},{"search_endpoint.label":pattern}]}},
            doc! {"$skip":offset as i64},
            doc! {"$limit":limit as i64},
            doc! {"$unset":"search_endpoint"},
        ])
        .max_time(std::time::Duration::from_secs(5))
        .await?
        .with_type::<UserService>()
        .try_collect()
        .await?)
}
