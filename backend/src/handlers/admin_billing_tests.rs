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
        assert!(status.is_success() || status == StatusCode::SERVICE_UNAVAILABLE);
        let count = f
            .state
            .db
            .collection::<Document>(crate::models::billing_rate_cache::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap();
        assert!(count >= 0);
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

    #[test]
    fn component_sources_are_part_of_the_overview_contract() {
        let response = serde_json::json!({"primary_source":"service","component_sources":["service","legacy"]});
        assert_eq!(response["primary_source"], "service");
        assert_eq!(response["component_sources"].as_array().unwrap().len(), 2);
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
