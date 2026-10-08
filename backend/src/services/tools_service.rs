use super::platform_key_service::{self, OwnerGrants};
use crate::{
    crypto::aes::EncryptionKeys,
    errors::AppResult,
    models::{
        downstream_service::{DownstreamService, OfferingKind},
        service_endpoint::{
            CostClass, DataScope, EndpointRisk, ExecutionKind, PublicationState, ServiceEndpoint,
        },
    },
};
use futures::TryStreamExt;
use mongodb::{Database, bson::doc};
use serde::Serialize;

#[derive(Serialize)]
pub struct ToolOperation {
    pub name: String,
    pub description: Option<String>,
    pub method: String,
    pub path: String,
    pub data_scope: Option<DataScope>,
    pub cost_class: Option<CostClass>,
    pub execution: ExecutionKind,
    pub risk: Option<EndpointRisk>,
}
#[derive(Serialize)]
pub struct ToolAccess {
    pub platform: bool,
    pub byok: bool,
}
#[derive(Serialize)]
pub struct ToolLimits {
    pub rate_limit_per_second: u32,
    pub burst: u32,
}
#[derive(Serialize)]
#[serde(untagged)]
pub enum ToolPrice {
    Lane {
        metric: crate::models::service_billing::BillingMetric,
        credits_per_unit: String,
    },
    Free(&'static str),
}
#[derive(Serialize)]
pub struct ToolPricing {
    pub platform: ToolPrice,
    pub byok: Option<ToolPrice>,
}
#[derive(Serialize)]
pub struct ToolOffering {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub offering_kind: OfferingKind,
    pub supplier: Option<String>,
    pub topics: Vec<String>,
    pub import_source: Option<crate::models::downstream_service::CatalogImportSource>,
    pub homepage_url: Option<String>,
    pub provider_label: String,
    pub access: ToolAccess,
    pub pricing: ToolPricing,
    pub limits: ToolLimits,
    pub operations: Vec<ToolOperation>,
    pub credential_configured: bool,
}

fn price(lane: Option<&crate::models::service_billing::LanePricing>) -> ToolPrice {
    let zero = |value: &str| {
        value.parse::<crate::models::credits::Credits>()
            == Ok(crate::models::credits::Credits::ZERO)
    };
    if lane.is_none_or(|lane| {
        zero(&lane.credits_per_unit)
            && lane
                .components
                .iter()
                .all(|component| zero(&component.credits_per_unit))
    }) {
        return ToolPrice::Free("free");
    }
    lane.map(|lane| ToolPrice::Lane {
        metric: lane.metric,
        credits_per_unit: lane.credits_per_unit.clone(),
    })
    .unwrap_or(ToolPrice::Free("free"))
}

pub async fn list(
    db: &Database,
    keys: &EncryptionKeys,
    actor_id: &str,
    admin: bool,
    limits: (u32, u32),
) -> AppResult<Vec<ToolOffering>> {
    let services: Vec<DownstreamService> = db
        .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .find(doc! {"offering_kind":"tool","is_active":true})
        .sort(doc! {"name":1})
        .await?
        .try_collect()
        .await?;
    let grants = OwnerGrants::load_for_listing(db, actor_id).await?;
    let providers = platform_key_service::load_providers(db).await?;
    let ids: Vec<_> = services.iter().map(|s| s.id.as_str()).collect();
    let endpoints: Vec<ServiceEndpoint> = db
        .collection::<ServiceEndpoint>(crate::models::service_endpoint::COLLECTION_NAME)
        .find(doc! {"service_id":{"$in":ids},"is_active":true,"$or":[{"publication":"published"},{"publication":{"$exists":false}}]})
        .await?
        .try_collect()
        .await?;
    let mut result = Vec::new();
    for service in services {
        let provider = service
            .provider_config_id
            .as_deref()
            .and_then(|id| providers.get(id));
        let platform =
            platform_key_service::available_with_grants(&service, provider, actor_id, &grants)
                || platform_key_service::no_auth_available_with_grants(&service, actor_id, &grants);
        if !platform && !admin {
            continue;
        }
        let configured =
            platform_key_service::credential_configured(keys, &service).await == Some(true);
        if !configured && service.auth_method != "none" && !admin {
            continue;
        }
        let operations: Vec<ToolOperation> = endpoints
            .iter()
            .filter(|e| e.service_id == service.id && e.publication == PublicationState::Published)
            .map(|e| ToolOperation {
                name: e.name.clone(),
                description: e.description.clone(),
                method: e.method.clone(),
                path: e.path.clone(),
                data_scope: e.data_scope,
                cost_class: e.cost_class,
                execution: e.execution,
                risk: e.risk,
            })
            .collect();
        if operations.is_empty() {
            continue;
        }
        let byok = service.auth_method != "none";
        let billing = service.billing.as_ref();
        let provider_label = provider
            .map(|p| p.name.clone())
            .or_else(|| service.supplier.clone())
            .unwrap_or_else(|| service.name.clone());
        result.push(ToolOffering {
            id: service.id,
            slug: service.slug,
            name: service.name,
            description: service.description,
            offering_kind: service.offering_kind,
            supplier: service.supplier,
            topics: service.topics,
            import_source: service.import_source,
            homepage_url: service.homepage_url,
            provider_label,
            access: ToolAccess { platform, byok },
            pricing: ToolPricing {
                platform: price(billing.and_then(|b| b.platform_key_pricing.as_ref())),
                byok: byok.then(|| price(billing.and_then(|b| b.byok_pricing.as_ref()))),
            },
            limits: ToolLimits {
                rate_limit_per_second: limits.0,
                burst: limits.1,
            },
            operations,
            credential_configured: configured,
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn only_published_ready_and_acl_visible_tools_are_listed() {
        let Some(db) = crate::test_utils::connect_test_database("tools_listing").await else {
            return;
        };
        let actor = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &actor,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.offering_kind = OfferingKind::Tool;
        service.service_category = "internal".into();
        service.auth_method = "none".into();
        service.requires_user_credential = false;
        db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
            .insert_one(&service)
            .await
            .unwrap();
        let input = super::super::service_endpoint_service::EndpointInput {
            name: "read".into(),
            description: None,
            method: "GET".into(),
            path: "/read".into(),
            target_id: None,
            parameters: None,
            request_body_schema: None,
            request_content_type: None,
            request_body_required: false,
            response_description: None,
            response: Default::default(),
            risk: Some(EndpointRisk::Read),
            supports_idempotency_key: false,
            data_scope: Some(DataScope::Public),
            cost_class: Some(CostClass::Free),
            execution: ExecutionKind::HttpOperation,
        };
        let endpoint =
            super::super::service_endpoint_service::create_endpoint(&db, &service.id, input)
                .await
                .unwrap();
        let keys = crate::test_utils::test_encryption_keys();
        assert!(
            list(&db, &keys, &actor, false, (2, 10))
                .await
                .unwrap()
                .is_empty()
        );
        db.collection::<ServiceEndpoint>(crate::models::service_endpoint::COLLECTION_NAME)
            .update_one(
                doc! {"_id":endpoint.id},
                doc! {"$set":{"$or":[{"publication":"published"},{"publication":{"$exists":false}}],"is_active":true}},
            )
            .await
            .unwrap();
        let rows = list(&db, &keys, &actor, false, (2, 10)).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].access.platform);
        assert!(!rows[0].access.byok);
        assert!(matches!(rows[0].pricing.platform, ToolPrice::Free("free")));
        db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME).update_one(doc! {"_id":service.id}, doc! {"$set":{"auth_method":"bearer","platform_key":{"enabled":true,"audience":"public","allowed_owner_ids":[]}}}).await.unwrap();
        assert!(
            list(&db, &keys, &actor, false, (2, 10))
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            list(&db, &keys, &actor, true, (2, 10)).await.unwrap().len(),
            1
        );
        db.drop().await.unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn free_price_requires_every_lane_component_to_be_free() {
        let mut lane: crate::models::service_billing::LanePricing = serde_json::from_value(
            serde_json::json!({"metric":"requests","credits_per_unit":"0","components":[]}),
        )
        .unwrap();
        assert!(matches!(price(Some(&lane)), ToolPrice::Free("free")));
        lane.components.push(
            serde_json::from_value(
                serde_json::json!({"metric":"input_tokens","credits_per_unit":"0.02"}),
            )
            .unwrap(),
        );
        assert!(matches!(price(Some(&lane)), ToolPrice::Lane { .. }));
        assert!(matches!(price(None), ToolPrice::Free("free")));
    }
}
