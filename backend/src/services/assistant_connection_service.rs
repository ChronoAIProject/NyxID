//! Safe connection identity and hosted setup guidance shared by assistant surfaces.
use bson::doc;
use mongodb::Database;
use serde_json::{Value, json};

use crate::{
    errors::{AppError, AppResult},
    models::{
        downstream_service::DownstreamService,
        user::{User, UserType},
        user_service::UserService,
    },
};

pub async fn resolve_catalog(
    db: &Database,
    actor: &str,
    id: Option<&str>,
    slug: Option<&str>,
) -> AppResult<DownstreamService> {
    let missing = || AppError::NotFound("Catalog entry not found".into());
    let mut filter = doc! {"is_active":true};
    match (id, slug) {
        (Some(id), slug) => {
            filter.insert("_id", id);
            if let Some(slug) = slug {
                filter.insert("slug", slug);
            }
        }
        (None, Some(slug)) => {
            filter.insert("slug", slug);
        }
        _ => {
            return Err(AppError::ValidationError(
                "service_id or service_slug is required".into(),
            ));
        }
    }
    let catalog = db
        .collection::<DownstreamService>("downstream_services")
        .find_one(filter)
        .await?
        .ok_or_else(missing)?;
    if super::retired_service_service::is_retired(&catalog) {
        return Err(missing());
    }
    super::catalog_service::enforce_catalog_read_access(db, actor, &catalog).await?;
    Ok(catalog)
}

pub async fn facts(db: &Database, actor: &str, connection: &UserService) -> AppResult<Value> {
    facts_for(db, actor, std::slice::from_ref(connection))
        .await?
        .remove(&connection.id)
        .ok_or_else(|| AppError::NotFound("Connection owner unavailable".into()))
}

/// Unsupported flows never advertise a link that cannot be completed personally.
pub async fn connect_own(db: &Database, actor: &str, connection: &UserService) -> AppResult<Value> {
    let Some(id) = connection.catalog_service_id.as_deref() else {
        return Ok(
            json!({"supported":false,"reason":"custom_connection","instructions":"Ask the owner to approve this exact organization operation, or use Plugins → Advanced setup for a personal custom connection."}),
        );
    };
    let catalog = match resolve_catalog(db, actor, Some(id), None).await {
        Ok(catalog) => catalog,
        Err(AppError::NotFound(_)) => {
            return Ok(
                json!({"supported":false,"reason":"catalog_unavailable","instructions":"Ask the owner to review this organization operation."}),
            );
        }
        Err(error) => return Err(error),
    };
    if catalog.service_type == "ssh" || !catalog.requires_user_credential {
        return Ok(
            json!({"supported":false,"reason":if catalog.service_type == "ssh" {"ssh_requires_advanced_setup"} else {"personal_account_not_supported"},"instructions":"Ask the owner to approve this exact organization operation, or use Plugins → Advanced setup."}),
        );
    }
    Ok(
        json!({"supported":true,"catalog_service_id":catalog.id,"service_slug":catalog.slug,
        "instructions":format!("Offer nyx__connect_service for {} so the owner can connect their own account; or ask whether to use the organization connection. Specialists must ask the owner to connect via NyxBot or Plugins, then grant the specialist. Connecting never grants access automatically.",catalog.slug)}),
    )
}

pub async fn org_refusal(
    db: &Database,
    chat: &super::assistant_acknowledgement_service::ChatAuthority,
    service: &UserService,
    mut refusal: Value,
) -> AppResult<Value> {
    let facts = facts(db, &chat.user_id, service).await?;
    let offer = connect_own(db, &chat.user_id, service).await?;
    refusal["organization_name"] = facts["organization_name"].clone();
    refusal["connection_id"] = json!(service.id);
    refusal["connection_slug"] = json!(service.slug);
    refusal["catalog_service_id"] = facts["catalog_service_id"].clone();
    refusal["service_slug"] = facts["catalog_service_slug"].clone();
    refusal["connection"] = facts;
    refusal["connect_own"] = offer.clone();
    if let Some(id) = refusal["acknowledgement_id"].as_str() {
        db.collection::<bson::Document>("assistant_acknowledgements").update_one(
            doc! {"_id":id,"user_id":&chat.user_id,"conversation_id":&chat.conversation_id,
                "api_key_id":&chat.api_key_id,"confirmation.reason":"org_connection"},
            doc! {"$set":{"connect_own":bson::to_bson(&offer).map_err(|_| AppError::Internal("Connection offer failed".into()))?}}).await?;
    }
    Ok(refusal)
}

#[allow(clippy::too_many_arguments)]
pub async fn hosted(
    db: &Database,
    actor: &str,
    catalog: &DownstreamService,
    frontend: &str,
    platform: bool,
    label: Option<&str>,
    requested_by: Option<&str>,
    scopes: &[String],
) -> AppResult<Value> {
    let locked = super::feature_flag_service::personal_flag_enabled(
        db,
        actor,
        super::feature_flag_service::HOSTED_CONNECT_INTENT_FLAG_KEY,
    )
    .await?;
    let created = super::connect_link_service::create(
        db,
        super::connect_link_service::CreateInput {
            user_id: actor.into(),
            service_slug: catalog.slug.clone(),
            use_platform_key: Some(platform),
            assistant_intent_locked: locked,
            scopes: scopes.to_vec(),
            label: label.map(str::to_owned),
            endpoint_url: None,
            requested_by: requested_by.map(str::to_owned),
            callback_url: None,
            ttl_secs: None,
            oauth_client_id: None,
        },
    )
    .await?;
    Ok(
        json!({"status":"pending_connection","connect_url":super::connect_link_service::build_connect_url(frontend,&created.raw_token)?,
        "connect_link_id":created.link.id,"expires_at":created.link.expires_at.to_rfc3339(),"service_id":catalog.id,"catalog_service_id":catalog.id,"service_slug":catalog.slug,"scopes":created.link.scopes,
        "instructions":"Open the hosted link to connect your account, then call nyx__wait_for_connection. A pending link grants no access."}),
    )
}

/// Batched, fail-soft connection facts keyed by `UserService.id`. A connection
/// whose owner or catalog cannot be read is omitted rather than failing the
/// listing that asked for it.
pub async fn facts_for(
    db: &Database,
    actor: &str,
    connections: &[UserService],
) -> AppResult<std::collections::HashMap<String, Value>> {
    use futures::TryStreamExt;
    use std::collections::{BTreeSet, HashMap};
    let mut result = HashMap::new();
    if connections.is_empty() {
        return Ok(result);
    }
    let owner_ids: BTreeSet<&str> = connections.iter().map(|c| c.user_id.as_str()).collect();
    let owners: HashMap<String, User> = db
        .collection::<User>("users")
        .find(doc! {"_id":{"$in":owner_ids.into_iter().collect::<Vec<_>>()},"is_active":true})
        .await?
        .try_collect::<Vec<_>>()
        .await?
        .into_iter()
        .map(|owner| (owner.id.clone(), owner))
        .collect();
    let catalog_ids: BTreeSet<&str> = connections
        .iter()
        .filter_map(|c| c.catalog_service_id.as_deref())
        .collect();
    let mut catalogs: HashMap<String, DownstreamService> = HashMap::new();
    if !catalog_ids.is_empty() {
        let rows: Vec<DownstreamService> = db
            .collection::<DownstreamService>("downstream_services")
            .find(doc! {"_id":{"$in":catalog_ids.into_iter().collect::<Vec<_>>()},"is_active":true})
            .await?
            .try_collect()
            .await?;
        let checks = rows.into_iter().map(|catalog| async move {
            let visible = !super::retired_service_service::is_retired(&catalog)
                && super::catalog_service::enforce_catalog_read_access(db, actor, &catalog)
                    .await
                    .is_ok();
            visible.then_some(catalog)
        });
        for catalog in futures::future::join_all(checks)
            .await
            .into_iter()
            .flatten()
        {
            catalogs.insert(catalog.id.clone(), catalog);
        }
    }
    for connection in connections {
        let Some(owner) = owners.get(&connection.user_id) else {
            continue;
        };
        let catalog = connection
            .catalog_service_id
            .as_deref()
            .and_then(|id| catalogs.get(id));
        result.insert(connection.id.clone(), project(actor, owner, catalog));
    }
    Ok(result)
}

fn project(actor: &str, owner: &User, catalog: Option<&DownstreamService>) -> Value {
    let org = owner.user_type == UserType::Org;
    json!({"catalog_service_id":catalog.map(|c| &c.id),
        "catalog_service_slug":catalog.map(|c| &c.slug),
        "owner_type":if org {"organization"} else {"person"},
        "relation":if org {"organization"} else if owner.id == actor {"self"} else {"shared_person"},
        "organization_name":org.then(|| owner.display_name.as_deref().or(owner.slug.as_deref()).unwrap_or(&owner.id)),
        "account":{"status":"unknown"}})
}

/// Facts for MCP tool services, keyed by `McpToolService.service_id`.
pub async fn mcp_facts(
    db: &Database,
    actor: &str,
    services: &[&super::mcp_service::McpToolService],
) -> AppResult<std::collections::HashMap<String, Value>> {
    use futures::TryStreamExt;
    let ids: Vec<(&str, &str)> = services
        .iter()
        .filter_map(|service| match &service.source {
            super::mcp_service::McpToolSource::UserManaged {
                user_service_id, ..
            } => Some((service.service_id.as_str(), user_service_id.as_str())),
            _ => None,
        })
        .collect();
    if ids.is_empty() {
        return Ok(Default::default());
    }
    let connections: Vec<UserService> = db
        .collection::<UserService>("user_services")
        .find(doc! {"_id":{"$in":ids.iter().map(|(_, id)| *id).collect::<Vec<_>>()}})
        .await?
        .try_collect()
        .await?;
    let by_connection = facts_for(db, actor, &connections).await?;
    Ok(ids
        .into_iter()
        .filter_map(|(service, connection)| {
            by_connection
                .get(connection)
                .map(|facts| (service.to_owned(), facts.clone()))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::assistant_authority_tests::{connected, orchestrator_fixture};
    use uuid::Uuid;

    #[tokio::test]
    async fn authorized_catalog_resolver_matches_slug_id_and_hides_mismatch_or_private_entry() {
        let f = orchestrator_fixture("assistant_catalog_resolution").await;
        crate::services::role_service::seed_system_roles(&f.state.db)
            .await
            .unwrap();
        let mut c = crate::models::downstream_service::test_helpers::dummy_service();
        c.id = Uuid::new_v4().to_string();
        c.slug = "public-github".into();
        c.visibility = "public".into();
        f.state
            .db
            .collection::<DownstreamService>("downstream_services")
            .insert_one(&c)
            .await
            .unwrap();
        for (id, slug) in [
            (Some(c.id.as_str()), None),
            (None, Some(c.slug.as_str())),
            (Some(c.id.as_str()), Some(c.slug.as_str())),
        ] {
            assert_eq!(
                resolve_catalog(&f.state.db, &f.owner, id, slug)
                    .await
                    .unwrap()
                    .id,
                c.id
            );
        }
        for (id, slug) in [
            (Some(c.id.as_str()), Some("mismatch")),
            (Some("absent"), None),
        ] {
            assert!(matches!(
                resolve_catalog(&f.state.db, &f.owner, id, slug).await,
                Err(AppError::NotFound(_))
            ));
        }
        c.id = Uuid::new_v4().to_string();
        c.slug = "private".into();
        c.visibility = "private".into();
        c.created_by = "other-person".into();
        c.owner_user_id = Some("other-person".into());
        f.state
            .db
            .collection::<DownstreamService>("downstream_services")
            .insert_one(&c)
            .await
            .unwrap();
        assert!(matches!(
            resolve_catalog(&f.state.db, &f.owner, Some(&c.id), None).await,
            Err(AppError::NotFound(_))
        ));
        f.state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn connection_facts_distinguish_self_shared_person_org_and_unsupported_connect_own() {
        let f = orchestrator_fixture("assistant_connection_facts").await;
        for (kind, relation) in [
            (UserType::Person, "self"),
            (UserType::Person, "shared_person"),
            (UserType::Org, "organization"),
        ] {
            let owner = if relation == "self" {
                f.owner.clone()
            } else {
                let id = Uuid::new_v4().to_string();
                f.state
                    .db
                    .collection::<User>("users")
                    .insert_one(crate::test_utils::test_user(&id, kind))
                    .await
                    .unwrap();
                id
            };
            let id = connected(&f.state.db, &owner, relation, "https://example.test").await;
            let connection =
                crate::services::user_service_service::find_user_service_by_id(&f.state.db, &id)
                    .await
                    .unwrap()
                    .unwrap();
            let result = facts(&f.state.db, &f.owner, &connection).await.unwrap();
            assert_eq!(result["relation"], relation);
            assert_eq!(result["account"]["status"], "unknown");
            assert_eq!(
                result["owner_type"],
                if relation == "organization" {
                    "organization"
                } else {
                    "person"
                }
            );
            assert_eq!(
                connect_own(&f.state.db, &f.owner, &connection)
                    .await
                    .unwrap()["reason"],
                "custom_connection"
            );
        }
        f.state.db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn batched_facts_skip_unreadable_owners_instead_of_failing() {
        let f = orchestrator_fixture("assistant_connection_facts_batch").await;
        let org = Uuid::new_v4().to_string();
        let mut inactive = crate::test_utils::test_user(&org, UserType::Org);
        inactive.is_active = false;
        f.state
            .db
            .collection::<User>("users")
            .insert_one(inactive)
            .await
            .unwrap();
        let own = connected(&f.state.db, &f.owner, "own", "https://example.test").await;
        let stale = connected(&f.state.db, &org, "stale-org", "https://example.test").await;
        let mut rows = Vec::new();
        for id in [&own, &stale] {
            rows.push(
                crate::services::user_service_service::find_user_service_by_id(&f.state.db, id)
                    .await
                    .unwrap()
                    .unwrap(),
            );
        }
        let facts = facts_for(&f.state.db, &f.owner, &rows).await.unwrap();
        assert_eq!(facts[&own]["relation"], "self");
        assert!(
            !facts.contains_key(&stale),
            "an inactive owner is omitted, not an error"
        );
        f.state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn connect_own_requires_visible_personal_catalog_and_reports_ssh_or_custom_as_unsupported()
     {
        let f = orchestrator_fixture("safe_connect_own_catalog").await;
        crate::services::role_service::seed_system_roles(&f.state.db)
            .await
            .unwrap();
        let id = connected(&f.state.db, &f.owner, "github", "https://example.test").await;
        let mut connection =
            super::super::user_service_service::find_user_service_by_id(&f.state.db, &id)
                .await
                .unwrap()
                .unwrap();
        let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
        catalog.id = Uuid::new_v4().to_string();
        catalog.slug = "personal-github".into();
        catalog.requires_user_credential = true;
        catalog.visibility = "public".into();
        f.state
            .db
            .collection::<DownstreamService>("downstream_services")
            .insert_one(&catalog)
            .await
            .unwrap();
        connection.catalog_service_id = Some(catalog.id.clone());
        let offer = connect_own(&f.state.db, &f.owner, &connection)
            .await
            .unwrap();
        assert_eq!(offer["supported"], true);
        assert_eq!(offer["catalog_service_id"], catalog.id);
        let identity = facts(&f.state.db, &f.owner, &connection).await.unwrap();
        assert_eq!(identity["catalog_service_id"], catalog.id);
        assert_eq!(identity["catalog_service_slug"], catalog.slug);
        f.state
            .db
            .collection::<bson::Document>("downstream_services")
            .update_one(
                doc! {"_id":&catalog.id},
                doc! {"$set":{"service_type":"ssh"}},
            )
            .await
            .unwrap();
        assert_eq!(
            connect_own(&f.state.db, &f.owner, &connection)
                .await
                .unwrap()["reason"],
            "ssh_requires_advanced_setup"
        );
        f.state.db.collection::<bson::Document>("downstream_services").update_one(doc! {"_id":&catalog.id},doc! {"$set":{"visibility":"private","owner_user_id":"another-person","created_by":"another-person"}}).await.unwrap();
        let unavailable = connect_own(&f.state.db, &f.owner, &connection)
            .await
            .unwrap();
        assert_eq!(unavailable["reason"], "catalog_unavailable");
        assert!(unavailable.get("catalog_service_id").is_none());
        assert!(unavailable.get("service_slug").is_none());
        f.state.db.drop().await.unwrap();
    }
}
