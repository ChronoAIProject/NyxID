use super::*;
use crate::{
    errors::AppError,
    models::user::{COLLECTION_NAME as USERS, User, UserType},
    services::role_service,
    test_utils,
};
use bson::doc;
use uuid::Uuid;

async fn setup() -> (AppState, AuthUser, AuthUser, AuthUser) {
    let db = test_utils::connect_transaction_test_database("platform_models_api").await;
    registry::ensure_indexes(&db).await.unwrap();
    test_utils::init_test_audit_chain_hmac_key();
    role_service::seed_system_roles(&db).await.unwrap();
    let roles = role_service::get_platform_role_ids(&db).await.unwrap();
    let mut auths = Vec::new();
    for role in [Some(roles.admin), Some(roles.operator), None] {
        let id = Uuid::new_v4().to_string();
        let mut user = test_utils::test_user(&id, UserType::Person);
        if let Some(role) = role {
            user.role_ids.push(role);
        }
        db.collection::<User>(USERS).insert_one(user).await.unwrap();
        auths.push(test_utils::test_auth_user(&id));
    }
    (
        test_utils::test_app_state(db),
        auths.remove(0),
        auths.remove(0),
        auths.remove(0),
    )
}
fn input() -> CreateRequest {
    CreateRequest {
        public_id: "nyxid/gemini-3-pro".into(),
        display_name: "Gemini 3 Pro".into(),
        description: "Gemini model".into(),
        family: ModelFamily::Gemini,
        capabilities: Default::default(),
    }
}

#[tokio::test]
async fn admin_writes_operator_reads_and_private_projection() {
    let (state, admin, operator, person) = setup().await;
    assert!(matches!(
        create(State(state.clone()), person.clone(), Json(input())).await,
        Err(AppError::Forbidden(_))
    ));
    assert!(matches!(
        create(State(state.clone()), operator.clone(), Json(input())).await,
        Err(AppError::Forbidden(_))
    ));
    let Json(model) = create(State(state.clone()), admin.clone(), Json(input()))
        .await
        .unwrap();
    assert!(
        admin_get(State(state.clone()), person, Path(model.id.clone()))
            .await
            .is_err()
    );
    let Json(view) = admin_get(State(state.clone()), operator, Path(model.id.clone()))
        .await
        .unwrap();
    assert!(!view.execution_ready);
    assert_eq!(view.revision, 1);
    let mut delegated = admin.clone();
    delegated.auth_method = AuthMethod::Delegated;
    assert!(
        admin_get(State(state.clone()), delegated, Path(model.id.clone()))
            .await
            .is_err()
    );
    let row = registry::get(&state.db, &model.id).await.unwrap();
    let public = serde_json::to_value(PublicModel::from(&row)).unwrap();
    let mut fields = public
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    fields.sort_unstable();
    assert_eq!(
        fields,
        vec![
            "availability",
            "capabilities",
            "description",
            "display_name",
            "id",
            "retail_prices"
        ]
    );
    for field in [
        "routes",
        "routing_revision_id",
        "billing_service_id",
        "family",
        "project",
        "location",
        "credential_service_id",
        "entitlements",
    ] {
        assert!(public.get(field).is_none());
    }
    let user_id = admin.user_id.to_string();
    let Json(explanation) = explain(
        State(state.clone()),
        admin,
        Path(model.id),
        Query(ExplainQuery { user_id }),
    )
    .await
    .unwrap();
    assert_eq!(explanation.policy_reason, "no_entitlement");
    assert!(!explanation.executable);
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn disabled_discovery_is_empty_and_does_not_create_rows() {
    let (state, admin, _, person) = setup().await;
    let _ = create(State(state.clone()), admin, Json(input()))
        .await
        .unwrap();
    let Json(result) = public_list(
        State(state.clone()),
        person.clone(),
        Query(ListQuery {
            after: None,
            limit: Some(10),
        }),
    )
    .await
    .unwrap();
    assert!(result.models.is_empty());
    assert!(result.next_after.is_none());
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(ROUTING_COLLECTION)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    for method in [
        AuthMethod::Relay,
        AuthMethod::ServiceAccount,
        AuthMethod::Delegated,
    ] {
        let mut auth = person.clone();
        auth.auth_method = method;
        auth.scope = "proxy".into();
        assert!(
            public_list(
                State(state.clone()),
                auth,
                Query(ListQuery {
                    after: None,
                    limit: None
                })
            )
            .await
            .is_err()
        );
    }
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn route_selectors_and_public_id_changes_are_closed_inputs() {
    assert!(serde_json::from_value::<CreateRequest>(serde_json::json!({"public_id":"nyxid/gemini-3-pro","display_name":"Gemini","family":"gemini","route":"bypass"})).is_err());
    assert!(serde_json::from_value::<UpdateRequest>(serde_json::json!({"expected_revision":1,"public_id":"nyxid/other","display_name":"Gemini","description":"","capabilities":{},"lifecycle":"draft","enabled":false})).is_err());
}

#[tokio::test]
async fn catalog_purpose_is_sticky_and_hidden_from_non_admins() {
    use crate::models::downstream_service::{
        DownstreamService, PlatformKeyConfig, PlatformKeyPurpose, test_helpers::dummy_service,
    };
    let (state, admin, _, person) = setup().await;
    crate::services::security_readiness_service::ready_test_replica(&state.db).await;
    let mut service = dummy_service();
    service.id = Uuid::new_v4().to_string();
    service.created_by = admin.user_id.to_string();
    service.visibility = "private".into();
    service.auth_method = "bearer".into();
    service.auth_key_name = "Authorization".into();
    service.service_category = "internal".into();
    service.requires_user_credential = false;
    service.is_active = true;
    service.provider_config_id = None;
    service.platform_key = Some(PlatformKeyConfig {
        purpose: PlatformKeyPurpose::ModelRoute,
        enabled: true,
        ..Default::default()
    });
    let collection = state
        .db
        .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME);
    collection.insert_one(&service).await.unwrap();
    assert!(
        crate::services::catalog_service::enforce_catalog_read_access(
            &state.db,
            &person.user_id.to_string(),
            &service
        )
        .await
        .is_err()
    );
    assert!(
        crate::services::catalog_service::enforce_catalog_read_access(
            &state.db,
            &admin.user_id.to_string(),
            &service
        )
        .await
        .is_err()
    );
    assert!(
        crate::handlers::services::get_service(
            State(state.clone()),
            admin.clone(),
            Path(service.id.clone())
        )
        .await
        .is_ok()
    );
    let mut delegated = admin.clone();
    delegated.auth_method = AuthMethod::Delegated;
    assert!(
        crate::handlers::services::get_service(
            State(state.clone()),
            delegated,
            Path(service.id.clone())
        )
        .await
        .is_err()
    );
    assert!(matches!(
        crate::handlers::endpoints::list_endpoints(
            State(state.clone()),
            person.clone(),
            Path(service.id.clone())
        )
        .await,
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(
        crate::handlers::services_helpers::resolve_service_or_user_service(
            &state,
            &service.id,
            &person.user_id.to_string()
        )
        .await,
        Err(AppError::NotFound(_))
    ));
    let body:crate::handlers::services::UpdateServiceRequest=serde_json::from_value(serde_json::json!({"platform_key":{"enabled":true,"audience":"restricted","allowed_owner_ids":[]}})).unwrap();
    assert!(
        crate::handlers::services::update_service(
            State(state.clone()),
            admin,
            Default::default(),
            Path(service.id.clone()),
            Json(body)
        )
        .await
        .is_err()
    );
    let live = collection
        .find_one(doc! {"_id":&service.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        live.platform_key.unwrap().purpose,
        PlatformKeyPurpose::ModelRoute
    );
    let filter =
        crate::services::catalog_service::visibility_filter(&person.user_id.to_string(), &[]);
    assert_eq!(collection.count_documents(filter).await.unwrap(), 0);
    state.db.drop().await.unwrap();
}

#[test]
fn platform_model_metadata_route_matches_billing_inventory() {
    use crate::services::billing::route_inventory::{BILLING_ROUTE_INVENTORY, BillingRoutePolicy};
    let mounted = crate::routes::mounted_billing_route_inventory();
    let row = mounted
        .iter()
        .find(|r| r.route == "/api/v1/llm/models")
        .unwrap();
    assert!(matches!(row.policy, BillingRoutePolicy::Exempt(_)));
    assert!(BILLING_ROUTE_INVENTORY.contains(row));
}

#[tokio::test]
async fn dormant_platform_model_never_falls_through_to_a_provider() {
    use crate::services::billing::{BillingIngress, route_inventory::BillingRoutePolicy};
    use axum::{body::Body, http::Request};
    let (state, admin, _, person) = setup().await;
    let _ = create(State(state.clone()), admin, Json(input()))
        .await
        .unwrap();
    for model in ["nyxid/gemini-3-pro", "nyxid/unknown"] {
        let mut request = Request::builder()
            .method("POST")
            .uri("/api/v1/llm/gateway/v1/chat/completions")
            .body(Body::from(
                serde_json::json!({"model":model,"messages":[]}).to_string(),
            ))
            .unwrap();
        request
            .extensions_mut()
            .insert(BillingRoutePolicy::Metered(BillingIngress::LlmGateway));
        assert!(matches!(
            crate::handlers::llm_gateway::gateway_request(
                State(state.clone()),
                person.clone(),
                Path("chat/completions".into()),
                request
            )
            .await,
            Err(AppError::NotFound(_))
        ));
    }
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn model_tariff_handlers_keep_writes_admin_only_and_reads_private() {
    use crate::services::billing::model_tariffs;
    let (state, admin, operator, person) = setup().await;
    crate::services::security_readiness_service::ready_test_replica(&state.db).await;
    let Json(model) = create(
        State(state.clone()),
        admin.clone(),
        Json(CreateRequest {
            public_id: "nyxid/gpt-test".into(),
            family: ModelFamily::Openai,
            ..input()
        }),
    )
    .await
    .unwrap();
    let (service, t, _) = model_tariffs::tests::fixture();
    state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>("downstream_services")
        .insert_one(&service)
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>(crate::models::platform_model::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&model.id},
            doc! {"$set":{"billing_service_id":&service.id}},
        )
        .await
        .unwrap();
    let body = model_tariffs::NewTariff {
        revision: model.revision,
        lane: t.lane,
        variant: t.variant,
        contract: t.contract,
        bounds: t.bounds,
        components: t.components,
        source: t.source,
    };
    assert!(
        tariffs(State(state.clone()), person, Path(model.id.clone()))
            .await
            .is_err()
    );
    assert!(
        tariffs(
            State(state.clone()),
            operator.clone(),
            Path(model.id.clone())
        )
        .await
        .unwrap()
        .0
        .is_empty()
    );
    let mut delegated = admin.clone();
    delegated.auth_method = AuthMethod::Delegated;
    assert!(
        tariffs(
            State(state.clone()),
            delegated.clone(),
            Path(model.id.clone())
        )
        .await
        .is_err()
    );
    for caller in [operator.clone(), delegated] {
        assert!(
            create_tariff(
                State(state.clone()),
                caller,
                Path(model.id.clone()),
                Json(body.clone())
            )
            .await
            .is_err()
        );
    }
    let Json(created) = create_tariff(
        State(state.clone()),
        admin.clone(),
        Path(model.id.clone()),
        Json(body),
    )
    .await
    .unwrap();
    assert_eq!(
        created.status,
        crate::models::platform_model_tariff::TariffStatus::Pending
    );
    assert_eq!(
        tariffs(
            State(state.clone()),
            operator.clone(),
            Path(model.id.clone())
        )
        .await
        .unwrap()
        .0
        .len(),
        1
    );
    assert!(
        retire_tariff(
            State(state.clone()),
            operator,
            Path((model.id.clone(), created.id.clone())),
            Json(RetireTariffRequest {
                revision: created.revision
            })
        )
        .await
        .is_err()
    );
    assert_eq!(
        retire_tariff(
            State(state.clone()),
            admin,
            Path((model.id, created.id)),
            Json(RetireTariffRequest {
                revision: created.revision
            })
        )
        .await
        .unwrap(),
        axum::http::StatusCode::NO_CONTENT
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn retail_projection_matches_billing_and_never_exposes_private_tariff_data() {
    use crate::{
        models::{
            billing_rate_cache::BillingRateCache, downstream_service::DownstreamService,
            platform_model_tariff::ModelTariff,
        },
        services::billing::model_tariffs,
    };
    let (state, admin, _, _) = setup().await;
    let Json(created) = create(
        State(state.clone()),
        admin,
        Json(CreateRequest {
            public_id: "nyxid/gpt-test".into(),
            family: ModelFamily::Openai,
            display_name: "GPT test".into(),
            ..input()
        }),
    )
    .await
    .unwrap();
    let mut model = registry::get(&state.db, &created.id).await.unwrap();
    let (mut service, mut tariff, rates) = model_tariffs::tests::fixture();
    model.billing_service_id = Some(service.id.clone());
    let previous_components = tariff.components.clone();
    tariff.model_id = model.id.clone();
    model_tariffs::validate(&mut tariff, &service.slug).unwrap();
    let rates: std::collections::HashMap<_, _> = tariff
        .components
        .iter()
        .map(|component| {
            let previous = previous_components
                .iter()
                .find(|p| p.key == component.key)
                .unwrap();
            let mut rate = rates[&previous.lago_metric_code].clone();
            rate.id = BillingRateCache::cache_id(&component.lago_metric_code, None);
            rate.lago_metric_code = component.lago_metric_code.clone();
            (component.lago_metric_code.clone(), rate)
        })
        .collect();
    service
        .billing
        .as_mut()
        .unwrap()
        .platform_key_pricing
        .as_mut()
        .unwrap()
        .model_prices[0]
        .model_id = model.id.clone();
    // An unrequested model sharing the same billing service must not trigger a
    // private tariff read, even if its row cannot be decoded by this replica.
    service
        .billing
        .as_mut()
        .unwrap()
        .platform_key_pricing
        .as_mut()
        .unwrap()
        .model_prices
        .push(crate::models::platform_model_tariff::ModelPriceBinding {
            model_id: Uuid::new_v4().to_string(),
            allowance_metrics: vec![],
            variant: Default::default(),
            active_tariff_id: Some("unrequested-private-tariff".into()),
            pending_tariff_id: None,
        });
    state
        .db
        .collection::<bson::Document>("platform_model_tariffs")
        .insert_one(doc! {"_id":"unrequested-private-tariff","status":"unknown-future-version"})
        .await
        .unwrap();
    state
        .db
        .collection::<DownstreamService>("downstream_services")
        .insert_one(&service)
        .await
        .unwrap();
    state
        .db
        .collection::<ModelTariff>("platform_model_tariffs")
        .insert_one(&tariff)
        .await
        .unwrap();
    state
        .db
        .collection::<BillingRateCache>("billing_rate_cache")
        .insert_many(rates.values())
        .await
        .unwrap();
    let mut results =
        registry::retail_explanations(&state.db, std::slice::from_ref(&model), 900, true)
            .await
            .unwrap();
    let actual: Vec<RetailPrice> = results
        .remove(&model.id)
        .unwrap()
        .into_iter()
        .map(Into::into)
        .collect();
    let expected: Vec<RetailPrice> = model_tariffs::explain(
        &service,
        std::slice::from_ref(&tariff),
        &rates,
        chrono::Utc::now(),
        900,
        true,
    )
    .into_iter()
    .filter(|p| p.model_id == model.id)
    .map(Into::into)
    .collect();
    let actual = serde_json::to_value(actual).unwrap();
    assert_eq!(actual, serde_json::to_value(expected).unwrap());
    assert_eq!(actual[0]["available"], true);
    assert_eq!(
        actual[0]["components"][0]["credits_per_unit"],
        "0.000001000001"
    );
    let json = actual.to_string();
    for (index, private_field) in [
        &service.id,
        &tariff.id,
        &tariff.source,
        &tariff.components[0].lago_metric_code,
        "fixture_sha256",
        "max_quantity",
        "sync_error",
        "service_id",
        "model_id",
    ]
    .into_iter()
    .enumerate()
    {
        assert!(
            !json.contains(private_field),
            "private field #{index} leaked into the public projection"
        );
    }
    // Missing/stale economics never appear as free or fall back to 999 base credits.
    state
        .db
        .collection::<BillingRateCache>("billing_rate_cache")
        .delete_many(doc! {})
        .await
        .unwrap();
    let mut result =
        registry::retail_explanations(&state.db, std::slice::from_ref(&model), 900, true)
            .await
            .unwrap();
    let price = RetailPrice::from(result.remove(&model.id).unwrap().remove(0));
    assert!(!price.available);
    assert!(price.components.is_empty());
    state.db.drop().await.unwrap();
}

#[test]
fn unavailable_retail_price_hides_components_even_when_billing_is_disabled() {
    use crate::services::billing::model_tariffs;
    let (service, tariff, rates) = model_tariffs::tests::fixture();
    let price = RetailPrice::from(
        model_tariffs::explain(&service, &[tariff], &rates, chrono::Utc::now(), 900, false)
            .remove(0),
    );
    assert!(!price.available);
    assert!(price.components.is_empty());
}

#[tokio::test]
async fn public_retail_metadata_is_projected_only_after_live_entitlement_and_flag_checks() {
    use crate::{
        models::{downstream_service::DownstreamService, platform_model_tariff::ModelTariff},
        services::billing::model_tariffs,
    };
    let (state, admin, _, person) = setup().await;
    let Json(created) = create(
        State(state.clone()),
        admin,
        Json(CreateRequest {
            public_id: "nyxid/gpt-test".into(),
            family: ModelFamily::Openai,
            ..input()
        }),
    )
    .await
    .unwrap();
    let mut model = registry::get(&state.db, &created.id).await.unwrap();
    let (mut service, mut tariff, _) = model_tariffs::tests::fixture();
    service.platform_key.as_mut().unwrap().enabled = true;
    model.billing_service_id = Some(service.id.clone());
    model.lifecycle = Lifecycle::Active;
    model.enabled = true;
    tariff.model_id = model.id.clone();
    model_tariffs::validate(&mut tariff, &service.slug).unwrap();
    service
        .billing
        .as_mut()
        .unwrap()
        .platform_key_pricing
        .as_mut()
        .unwrap()
        .model_prices[0]
        .model_id = model.id.clone();
    state
        .db
        .collection::<DownstreamService>("downstream_services")
        .insert_one(&service)
        .await
        .unwrap();
    state
        .db
        .collection::<ModelTariff>("platform_model_tariffs")
        .insert_one(&tariff)
        .await
        .unwrap();
    state
        .db
        .collection::<PlatformModel>(COLLECTION_NAME)
        .replace_one(doc! {"_id":&model.id}, &model)
        .await
        .unwrap();
    let read = || {
        public_list(
            State(state.clone()),
            person.clone(),
            Query(ListQuery {
                after: None,
                limit: Some(50),
            }),
        )
    };
    // Test-only activation bypasses the future execution gate, as registry ACL tests do.
    state.db.collection::<bson::Document>(crate::models::feature_flag_override::COLLECTION_NAME).insert_one(doc! {
        "_id":Uuid::new_v4().to_string(),"org_user_id":bson::Bson::Null,"flag_key":registry::FLAG,"target_kind":"global","target_key":bson::Bson::Null,
        "enabled":true,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),"updated_by":person.user_id.to_string()
    }).await.unwrap();
    assert!(read().await.unwrap().0.models.is_empty());
    state
        .db
        .collection::<bson::Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id":&model.id},
            doc! {"$set":{"entitlements.all_people":true}},
        )
        .await
        .unwrap();
    let result = read().await.unwrap().0;
    assert_eq!(result.models.len(), 1);
    assert_eq!(result.models[0].id, "nyxid/gpt-test");
    assert_eq!(result.models[0].retail_prices.len(), 1);
    assert!(!result.models[0].retail_prices[0].available);
    assert!(result.models[0].retail_prices[0].components.is_empty());
    state
        .db
        .collection::<bson::Document>(crate::models::feature_flag_override::COLLECTION_NAME)
        .update_many(doc! {}, doc! {"$set":{"enabled":false}})
        .await
        .unwrap();
    assert!(read().await.unwrap().0.models.is_empty());
    state.db.drop().await.unwrap();
}
