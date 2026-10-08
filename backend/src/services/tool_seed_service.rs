use crate::{
    errors::AppResult,
    models::{
        downstream_service::{
            DownstreamService, OfferingKind, PlatformKeyAudience, PlatformKeyConfig,
        },
        service_endpoint::{COLLECTION_NAME as ENDPOINTS, ServiceEndpoint},
    },
    services::{catalog_spec_registry, catalog_spec_sync, service_endpoint_service},
};
use mongodb::{Database, bson::doc};

pub async fn seed(db: &Database) -> AppResult<()> {
    let coll =
        db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME);
    let Some(template) = coll.find_one(doc! {"slug":"api-twitter"}).await? else {
        return Ok(());
    };
    for (slug, name, supplier, base_url, topic, auth_method, auth_key_name) in [
        (
            "tools-x",
            "X public data",
            "X Corp",
            "https://api.x.com/2",
            "social",
            "bearer",
            "Authorization",
        ),
        (
            "tools-tinyfish-search",
            "TinyFish Search",
            "TinyFish",
            "https://api.search.tinyfish.ai",
            "web-search",
            "header",
            "X-API-Key",
        ),
        (
            "tools-tinyfish-fetch",
            "TinyFish Fetch",
            "TinyFish",
            "https://api.fetch.tinyfish.ai",
            "page-fetch",
            "header",
            "X-API-Key",
        ),
    ] {
        if coll.find_one(doc! {"slug":slug}).await?.is_some() {
            continue;
        }
        let mut service = template.clone();
        service.id = uuid::Uuid::new_v4().to_string();
        service.slug = slug.into();
        service.name = name.into();
        service.description = Some(format!("{name} provided by NyxID"));
        service.offering_kind = OfferingKind::Tool;
        service.service_category = "internal".into();
        service.requires_user_credential = false;
        service.base_url = base_url.into();
        service.auth_method = auth_method.into();
        service.auth_key_name = auth_key_name.into();
        service.provider_config_id = None;
        service.credential_encrypted.clear();
        service.platform_key = Some(PlatformKeyConfig {
            enabled: true,
            audience: PlatformKeyAudience::Public,
            allowed_owner_ids: Vec::new(),
        });
        service.topics = vec![topic.into()];
        if slug == "tools-x" {
            service.topics.push("web-search".into());
        }
        service.supplier = Some(supplier.into());
        service.import_source = None;
        service.openapi_spec_url = catalog_spec_registry::spec_path_for_slug(slug).map(|path| {
            format!(
                "{}{}",
                std::env::var("BASE_URL")
                    .unwrap_or_else(|_| "http://localhost:3001".into())
                    .trim_end_matches('/'),
                path
            )
        });
        service.visibility = "public".into();
        service.created_by = "system".into();
        service.owner_user_id = None;
        service.is_active = true;
        service.billing = None;
        service.inference = None;
        service.inference_admin_modified = false;
        service.git_http = None;
        service.oauth_client_id = None;
        service.destination_targets.clear();
        service.default_request_headers = None;
        service.ws_frame_injections.clear();
        service.anonymous_endpoints.clear();
        service.proxy_operation_policy = None;
        service.identity_propagation_mode = "none".into();
        service.identity_include_user_id = false;
        service.identity_include_email = false;
        service.identity_include_name = false;
        service.identity_jwt_audience = None;
        service.forward_access_token = false;
        service.inject_delegation_token = false;
        service.token_exchange_config = None;
        service.developer_app_ids = None;
        service.created_at = chrono::Utc::now();
        service.updated_at = service.created_at;
        coll.insert_one(&service).await?;
        let spec = catalog_spec_registry::spec_for_slug(slug).expect("registered tool seed");
        let inputs = catalog_spec_sync::destination_endpoint_inputs(&service, &spec)?;
        let rows =
            service_endpoint_service::upsert_endpoints_additive(db, &service.id, inputs).await?;
        for row in rows {
            let published = slug != "tools-x"
                || [
                    "search_recent_tweets",
                    "get_user_by_username",
                    "get_user_tweets",
                ]
                .contains(&row.name.as_str());
            if published {
                db.collection::<ServiceEndpoint>(ENDPOINTS).update_one(doc! {"_id":&row.id},doc! {"$set":{"publication":"published","is_active":true,"data_scope":"public","cost_class":if slug == "tools-x" {"metered"} else {"free"},"execution":"http_operation"}}).await?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::TryStreamExt;
    #[tokio::test]
    async fn initial_tools_are_curated_and_later_startups_preserve_publication() {
        let db = crate::test_utils::connect_test_database("tools_seed")
            .await
            .unwrap();
        let mut template = crate::models::downstream_service::test_helpers::dummy_service();
        template.slug = "api-twitter".into();
        db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
            .insert_one(template)
            .await
            .unwrap();
        seed(&db).await.unwrap();
        let x = db
            .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
            .find_one(doc! {"slug":"tools-x"})
            .await
            .unwrap()
            .unwrap();
        let rows: Vec<ServiceEndpoint> = db
            .collection::<ServiceEndpoint>(ENDPOINTS)
            .find(doc! {"service_id":&x.id})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(rows.len(), 9);
        assert_eq!(rows.iter().filter(|row| row.is_active).count(), 3);
        let selected = rows
            .iter()
            .find(|row| row.name == "search_recent_tweets")
            .unwrap();
        db.collection::<ServiceEndpoint>(ENDPOINTS)
            .update_one(
                doc! {"_id":&selected.id},
                doc! {"$set":{"publication":"paused","is_active":false}},
            )
            .await
            .unwrap();
        seed(&db).await.unwrap();
        let preserved = db
            .collection::<ServiceEndpoint>(ENDPOINTS)
            .find_one(doc! {"_id":&selected.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            preserved.publication,
            crate::models::service_endpoint::PublicationState::Paused
        );
        assert!(!preserved.is_active);
        let state = crate::test_utils::test_app_state(db.clone());
        let tools = super::super::tools_service::list(
            &db,
            &state.encryption_keys,
            &uuid::Uuid::new_v4().to_string(),
            false,
            (10, 20),
        )
        .await
        .unwrap();
        assert!(
            tools.is_empty(),
            "unconfigured seed credentials must stay hidden"
        );
        db.drop().await.unwrap();
    }
}
