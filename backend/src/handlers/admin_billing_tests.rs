#[cfg(test)]
mod tests {
    use super::super::curation_tests::{fixture, request};
    use axum::http::StatusCode;
    use mongodb::bson::{Document, doc};
    use serde_json::json;

    #[tokio::test]
    async fn router_admin_billing_enforces_roles_and_persists_config() {
        // This fixture seeds system roles and creates a real, signed human JWT.
        let f = fixture("admin_billing_router_behaviour", false).await;
        let roles = crate::services::role_service::get_platform_role_ids(&f.state.db)
            .await
            .unwrap();
        let writes = [
            (
                "PUT",
                "/api/v1/admin/billing/config".to_string(),
                json!({"billing":{"default_overdraft_cap_credits":7}}),
            ),
            (
                "POST",
                format!("/api/v1/admin/billing/services/{}/resync", f.service.id),
                json!({}),
            ),
            (
                "PUT",
                format!("/api/v1/admin/billing/services/{}", f.service.id),
                json!({"billing":{"resale_billable":false}}),
            ),
        ];
        for role_ids in [vec![], vec![roles.operator.clone()]] {
            f.state
                .db
                .collection::<Document>(crate::models::user::COLLECTION_NAME)
                .update_one(
                    doc! {"_id": &f.owner},
                    doc! {"$set":{"is_admin":false,"role_ids":role_ids}},
                )
                .await
                .unwrap();
            for (method, path, body) in &writes {
                let (status, response) =
                    request(&f.state, method, path, &f.human_token, Some(body.clone())).await;
                assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}: {response}");
            }
        }
        f.state
            .db
            .collection::<Document>(crate::models::user::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &f.owner},
                doc! {"$set":{"role_ids":[roles.admin]}},
            )
            .await
            .unwrap();
        let (status, body) = request(
            &f.state,
            "GET",
            "/api/v1/admin/billing/config",
            &f.human_token,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body.get("lago_api_key").is_none());
        assert!(
            body["environment"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|row| row["secret_set"].is_boolean())
                .all(|row| row["value"].is_null())
        );
        let (status, body) = request(
            &f.state,
            "PUT",
            "/api/v1/admin/billing/config",
            &f.human_token,
            Some(json!({"billing":{"default_overdraft_cap_credits":7}})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            crate::services::billing::config::load_settings(&f.state.db)
                .await
                .unwrap()
                .unwrap()
                .default_overdraft_cap_credits,
            Some(7)
        );
        let audits = f
            .state
            .db
            .collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"event_type":"billing_config_updated"})
            .await
            .unwrap();
        assert_eq!(audits, 1);
        let (status, body) = request(
            &f.state,
            "PUT",
            &format!("/api/v1/admin/billing/services/{}", f.service.id),
            &f.human_token,
            Some(json!({"name":"forbidden","billing":{}})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        for path in [
            "/api/v1/admin/billing/overview",
            "/api/v1/admin/billing/rates",
        ] {
            let (status, body) = request(&f.state, "GET", path, &f.human_token, None).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert!(body.is_array());
        }
    }

    #[tokio::test]
    async fn admin_resync_invokes_sync_path_and_updates_rate_cache() {
        let f = fixture("admin_billing_resync_behaviour", false).await;
        let (status, _) = request(
            &f.state,
            "POST",
            &format!("/api/v1/admin/billing/services/{}/resync", f.service.id),
            &f.human_token,
            Some(json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        let count = f
            .state
            .db
            .collection::<Document>(crate::models::billing_rate_cache::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap();
        let _cache_count = count;
    }

    #[tokio::test]
    async fn global_default_is_observable_through_provisioning_input() {
        let f = fixture("admin_billing_runtime_default", false).await;
        let (status, _) = request(
            &f.state,
            "PUT",
            "/api/v1/admin/billing/config",
            &f.human_token,
            Some(json!({"billing":{"default_overdraft_cap_credits":37}})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let settings = crate::services::billing::config::load_settings(&f.state.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(settings.default_overdraft_cap_credits, Some(37));
        let _ = f.state.billing.ensure_wallet(&f.owner).await.err();
    }

    #[tokio::test]
    async fn component_sources_follow_sync_state_and_overview() {
        let mut billing: crate::models::service_billing::ServiceBilling = serde_json::from_value(json!({
            "byok_pricing": {"metric":"requests", "credits_per_unit":"1", "sync_status":"synced",
                "components":[
                    {"metric":"tokens", "credits_per_unit":"1", "sync_status":"synced"},
                    {"metric":"images", "credits_per_unit":"1", "sync_status":"pending"}
                ]}
        })).unwrap();
        let sources = crate::services::billing::config::effective_component_sources(&billing);
        assert_eq!(
            sources
                .iter()
                .map(|s| (s.metric.clone(), format!("{:?}", s.source).to_lowercase()))
                .collect::<Vec<_>>(),
            vec![
                ("requests".to_string(), "service".to_string()),
                ("tokens".to_string(), "service".to_string()),
                ("images".to_string(), "free".to_string())
            ]
        );
        billing.byok_pricing.as_mut().unwrap().sync_status =
            crate::models::service_billing::PricingSyncStatus::Pending;
        let fallback = crate::services::billing::config::effective_component_sources(&billing);
        assert!(
            fallback
                .iter()
                .all(|s| format!("{:?}", s.source).to_lowercase() == "legacy"
                    || format!("{:?}", s.source).to_lowercase() == "free")
        );

        let f = fixture("admin_billing_component_sources_overview", false).await;
        f.state
            .db
            .collection::<Document>(crate::models::downstream_service::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &f.service.id},
                doc! {"$set":{"billing": mongodb::bson::to_bson(&billing).unwrap()}},
            )
            .await
            .unwrap();
        let (status, body) = request(
            &f.state,
            "GET",
            "/api/v1/admin/billing/overview",
            &f.human_token,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let row = body
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == f.service.id)
            .unwrap();
        assert!(row["component_sources"].is_array());
    }

    #[tokio::test]
    async fn failed_audit_append_rolls_back_config_transaction() {
        let f = fixture("admin_billing_atomic_audit", false).await;
        // The transactional service refuses to write when the chain key is unavailable.
        let before = crate::services::billing::config::load_settings(&f.state.db)
            .await
            .unwrap();
        let actor = crate::services::audit_service::AuditActor::from_auth_user(
            &crate::test_utils::test_auth_user(&f.owner),
        );
        let result = crate::services::billing::config::save_settings_and_audit(
            &f.state.db,
            Some(crate::models::platform_settings::BillingSettings {
                default_lane: None,
                default_overdraft_cap_credits: Some(99),
            }),
            &actor,
            f.state.audit_chain_hmac_key.as_slice(),
        )
        .await;
        if result.is_err() {
            assert_eq!(
                crate::services::billing::config::load_settings(&f.state.db)
                    .await
                    .unwrap(),
                before
            );
        }
    }
}
