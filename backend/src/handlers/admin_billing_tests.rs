#[cfg(test)]
mod tests {
    use super::super::curation_tests::{Fixture, fixture, request};
    use axum::http::StatusCode;
    use mongodb::bson::{Document, doc};
    use serde_json::{Value, json};

    /// Effective prices only charge on billing-enabled deployments.
    fn with_billing_enabled(mut f: Fixture) -> Fixture {
        let mut config = f.state.config.clone();
        config.billing_enabled = true;
        f.state = crate::test_utils::test_app_state_with_config(f.state.db.clone(), config);
        f
    }

    async fn set_roles(f: &Fixture, is_admin: bool, role_ids: Vec<String>) {
        f.state
            .db
            .collection::<Document>(crate::models::user::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &f.owner},
                doc! {"$set": {"is_admin": is_admin, "role_ids": role_ids}},
            )
            .await
            .unwrap();
    }

    async fn audit_rows(f: &Fixture, event_type: &str) -> Vec<Document> {
        use futures::TryStreamExt;
        f.state
            .db
            .collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .find(doc! {"event_type": event_type})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap()
    }

    async fn insert_service(f: &Fixture, slug: &str, billing: Option<Value>) -> String {
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.id = uuid::Uuid::new_v4().to_string();
        service.slug = slug.to_string();
        service.name = slug.to_uppercase();
        service.created_by = f.owner.clone();
        service.billing = billing.map(|value| serde_json::from_value(value).unwrap());
        f.state
            .db
            .collection::<crate::models::downstream_service::DownstreamService>(
                crate::models::downstream_service::COLLECTION_NAME,
            )
            .insert_one(&service)
            .await
            .unwrap();
        service.id
    }

    async fn stored_billing(f: &Fixture, id: &str) -> Document {
        f.state
            .db
            .collection::<Document>(crate::models::downstream_service::COLLECTION_NAME)
            .find_one(doc! {"_id": id})
            .await
            .unwrap()
            .unwrap()
            .get_document("billing")
            .unwrap()
            .clone()
    }

    #[tokio::test]
    async fn writes_are_admin_only_while_operators_can_read() {
        let f = fixture("admin_billing_role_gates", false).await;
        let roles = crate::services::role_service::get_platform_role_ids(&f.state.db)
            .await
            .unwrap();
        let writes = [
            (
                "PUT",
                "/api/v1/admin/billing/config".to_string(),
                json!({"default_overdraft_cap_credits": 7}),
            ),
            (
                "POST",
                format!("/api/v1/admin/billing/services/{}/resync", f.service.id),
                json!({}),
            ),
            (
                "PUT",
                format!("/api/v1/admin/billing/services/{}", f.service.id),
                json!({"billing": {"resale_billable": false}}),
            ),
        ];
        let reads = [
            "/api/v1/admin/billing/config".to_string(),
            "/api/v1/admin/billing/rates".to_string(),
            "/api/v1/admin/billing/overview".to_string(),
            format!("/api/v1/admin/billing/services/{}", f.service.id),
        ];
        // A plain user can neither read nor write.
        set_roles(&f, false, vec![]).await;
        for path in &reads {
            let (status, body) = request(&f.state, "GET", path, &f.human_token, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "GET {path}: {body}");
        }
        // An operator reads every view but cannot change anything.
        set_roles(&f, false, vec![roles.operator.clone()]).await;
        for path in &reads {
            let (status, body) = request(&f.state, "GET", path, &f.human_token, None).await;
            assert_eq!(status, StatusCode::OK, "GET {path}: {body}");
        }
        for (method, path, body) in &writes {
            let (status, response) =
                request(&f.state, method, path, &f.human_token, Some(body.clone())).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}: {response}");
        }
        assert!(
            crate::services::billing::config::load_settings(&f.state.db)
                .await
                .unwrap()
                .is_none()
        );
        assert!(audit_rows(&f, "billing_config_updated").await.is_empty());
    }

    #[tokio::test]
    async fn config_put_sets_clears_and_keeps_fields_with_one_audit_each() {
        let f = fixture("admin_billing_config_patch", false).await;
        let put = |body: Value| {
            let state = f.state.clone();
            let token = f.human_token.clone();
            async move {
                request(
                    &state,
                    "PUT",
                    "/api/v1/admin/billing/config",
                    &token,
                    Some(body),
                )
                .await
            }
        };
        let (status, body) = put(json!({"default_overdraft_cap_credits": 7})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["default_overdraft_cap_credits"]["effective"], 7);
        assert_eq!(body["default_overdraft_cap_credits"]["source"], "global");
        assert_eq!(body["default_overdraft_cap_credits"]["global"], 7);
        assert_eq!(
            crate::services::billing::config::load_settings(&f.state.db)
                .await
                .unwrap()
                .unwrap()
                .default_overdraft_cap_credits,
            Some(7)
        );

        // An empty patch, an unknown field and an out-of-range cap change nothing.
        let (status, body) = put(json!({})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        let (status, body) = put(json!({"default_lane": "byok"})).await;
        assert!(status.is_client_error(), "{status} {body}");
        let (status, body) = put(json!({"default_overdraft_cap_credits": -1})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(
            body.to_string().contains("between 0 and 1000000000000"),
            "{body}"
        );
        assert_eq!(audit_rows(&f, "billing_config_updated").await.len(), 1);

        // null clears back to the environment default.
        let (status, body) = put(json!({"default_overdraft_cap_credits": null})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["default_overdraft_cap_credits"]["source"], "env");
        assert_eq!(
            body["default_overdraft_cap_credits"]["effective"],
            f.state.config.billing_default_overdraft_cap_credits
        );
        assert!(body["default_overdraft_cap_credits"]["global"].is_null());
        let audits = audit_rows(&f, "billing_config_updated").await;
        assert_eq!(audits.len(), 2);
        let last = audits
            .iter()
            .find(|row| {
                row.get_document("event_data")
                    .unwrap()
                    .get_document("default_overdraft_cap_credits")
                    .unwrap()
                    .get("after")
                    == Some(&mongodb::bson::Bson::Null)
            })
            .expect("clear audit row");
        let before = last
            .get_document("event_data")
            .unwrap()
            .get_document("default_overdraft_cap_credits")
            .unwrap()
            .get("before")
            .cloned();
        assert!(
            matches!(
                before,
                Some(mongodb::bson::Bson::Int32(7)) | Some(mongodb::bson::Bson::Int64(7))
            ),
            "{before:?}"
        );
    }

    #[tokio::test]
    async fn config_response_never_contains_secret_values() {
        let f = fixture("admin_billing_redaction", false).await;
        let mut config = f.state.config.clone();
        config.lago_api_url = Some("https://lago.example.test".into());
        config.lago_api_key = Some("lago-api-key-SECRET-1234".into());
        config.lago_webhook_secret = Some("lago-webhook-SECRET-5678".into());
        config.billing_ledger_hmac_key = Some("ab".repeat(32));
        let state = crate::test_utils::test_app_state_with_config(f.state.db.clone(), config);
        let (status, body) = request(
            &state,
            "GET",
            "/api/v1/admin/billing/config",
            &f.human_token,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let raw = body.to_string();
        for secret in [
            "lago-api-key-SECRET-1234",
            "lago-webhook-SECRET-5678",
            &"ab".repeat(32),
        ] {
            assert!(!raw.contains(secret), "secret leaked: {raw}");
        }
        let environment = body["environment"].as_array().unwrap();
        for key in [
            "LAGO_API_KEY",
            "LAGO_WEBHOOK_SECRET",
            "BILLING_LEDGER_HMAC_KEY",
        ] {
            let row = environment
                .iter()
                .find(|row| row["key"] == key)
                .unwrap_or_else(|| panic!("{key} missing"));
            assert_eq!(row["secret_set"], true, "{key}");
            assert!(row["value"].is_null(), "{key}");
            assert_eq!(row["source"], "env");
        }
        let url = environment
            .iter()
            .find(|row| row["key"] == "LAGO_API_URL")
            .unwrap();
        assert_eq!(url["value"], "https://lago.example.test");
        assert_eq!(body["lago_configured"], true);
    }

    #[tokio::test]
    async fn failed_audit_append_rolls_back_the_config_write() {
        let f = fixture("admin_billing_atomic_audit", false).await;
        let actor = crate::services::audit_service::AuditActor::from_auth_user(
            &crate::test_utils::test_auth_user(&f.owner),
        );
        let patch = crate::services::billing::config::GlobalBillingPatch {
            default_overdraft_cap_credits: Some(Some(99)),
        };
        // Make the audit insert fail inside the transaction.
        f.state
            .db
            .run_command(doc! {
                "collMod": crate::models::audit_log::COLLECTION_NAME,
                "validator": {"event_type": {"$ne": "billing_config_updated"}},
                "validationAction": "error",
            })
            .await
            .unwrap();
        crate::services::billing::config::update_global_defaults(
            &f.state.db,
            &patch,
            &actor,
            f.state.audit_chain_hmac_key.as_slice(),
        )
        .await
        .expect_err("a rejected audit row must abort the write");
        assert_eq!(
            crate::services::billing::config::load_settings(&f.state.db)
                .await
                .unwrap(),
            None
        );
        assert!(audit_rows(&f, "billing_config_updated").await.is_empty());

        // Positive control: the same write commits once the audit can append.
        f.state
            .db
            .run_command(doc! {
                "collMod": crate::models::audit_log::COLLECTION_NAME,
                "validator": {},
            })
            .await
            .unwrap();
        crate::services::billing::config::update_global_defaults(
            &f.state.db,
            &patch,
            &actor,
            f.state.audit_chain_hmac_key.as_slice(),
        )
        .await
        .unwrap();
        assert_eq!(
            crate::services::billing::config::load_settings(&f.state.db)
                .await
                .unwrap()
                .unwrap()
                .default_overdraft_cap_credits,
            Some(99)
        );
        assert_eq!(audit_rows(&f, "billing_config_updated").await.len(), 1);
    }

    #[tokio::test]
    async fn resync_without_lago_records_the_failed_sync_and_one_audit() {
        let f = with_billing_enabled(fixture("admin_billing_resync", false).await);
        let id = insert_service(
            &f,
            "resync-svc",
            Some(json!({
                "byok_pricing": {"metric": "requests", "credits_per_unit": "0.5",
                    "lago_metric_code": "platform_svc_resync-svc_byok", "sync_status": "pending"}
            })),
        )
        .await;
        let (status, body) = request(
            &f.state,
            "POST",
            &format!("/api/v1/admin/billing/services/{id}/resync"),
            &f.human_token,
            Some(json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["synced"], false);
        let billing = stored_billing(&f, &id).await;
        let lane = billing.get_document("byok_pricing").unwrap();
        assert_eq!(lane.get_str("sync_status").unwrap(), "failed");
        assert_eq!(
            lane.get_str("sync_error").unwrap(),
            "Lago is not configured; the reconcile sweep will retry"
        );
        let audits = audit_rows(&f, "billing_service_resynced").await;
        assert_eq!(audits.len(), 1);
        let data = audits[0].get_document("event_data").unwrap();
        assert_eq!(data.get_str("service_id").unwrap(), id);
        assert!(!data.get_bool("synced").unwrap());
        // The response already reflects the failed lane: own-key traffic is free
        // (no legacy opt-in) and the failed primary is listed as pending.
        let own = body["service"]["effective"]["classes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|g| {
                g["credential_classes"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("user_owned"))
            })
            .unwrap()
            .clone();
        assert_eq!(own["reason"], "lane_unsynced_free");
        assert_eq!(own["pending"][0]["sync_status"], "failed");

        let (status, _) = request(
            &f.state,
            "POST",
            "/api/v1/admin/billing/services/missing/resync",
            &f.human_token,
            Some(json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn overview_reports_per_value_prices_and_sources_sorted_by_slug() {
        let f = with_billing_enabled(fixture("admin_billing_overview_sources", false).await);
        insert_service(
            &f,
            "c-byok-synced",
            Some(
                json!({"byok_pricing": {"metric": "requests", "credits_per_unit": "0.25",
                "lago_metric_code": "c_byok", "sync_status": "synced",
                "components": [{"metric": "bytes", "credits_per_unit": "0.000001",
                    "lago_metric_code": "c_byok_bytes", "sync_status": "pending"}]}}),
            ),
        )
        .await;
        insert_service(
            &f,
            "a-unsynced",
            Some(json!({"platform_billable": true,
                "byok_pricing": {"metric": "tokens", "credits_per_unit": "0.002",
                    "lago_metric_code": "a_byok", "sync_status": "failed", "sync_error": "lago 500"}})),
        )
        .await;
        insert_service(&f, "b-legacy", Some(json!({"platform_billable": true}))).await;
        insert_service(&f, "d-unbilled", None).await;
        f.state
            .db
            .collection::<crate::models::billing_rate_cache::BillingRateCache>(
                crate::models::billing_rate_cache::COLLECTION_NAME,
            )
            .insert_one(crate::models::billing_rate_cache::BillingRateCache {
                id: crate::models::billing_rate_cache::BillingRateCache::cache_id(
                    "platform_requests",
                    None,
                ),
                lago_metric_code: "platform_requests".into(),
                model: None,
                credits_per_unit_micros: 1_500_000,
                credits_per_unit_pico: Some(1_500_000_000_000),
                synced_at: chrono::Utc::now(),
                retired_at: None,
            })
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
        let rows = body.as_array().unwrap();
        let slugs: Vec<&str> = rows.iter().map(|r| r["slug"].as_str().unwrap()).collect();
        assert_eq!(slugs, ["a-unsynced", "b-legacy", "c-byok-synced"]);
        let group = |row: &Value, class: &str| -> Value {
            row["effective"]["classes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|g| {
                    g["credential_classes"]
                        .as_array()
                        .unwrap()
                        .contains(&json!(class))
                })
                .unwrap()
                .clone()
        };

        // Unsynced primary: own-key traffic charges the legacy plan rate meanwhile.
        let own = group(&rows[0], "user_owned");
        assert_eq!(own["reason"], "lane_unsynced_legacy");
        assert_eq!(own["source"], "legacy");
        assert_eq!(own["prices"][0]["origin"], "legacy_plan");
        assert_eq!(own["prices"][0]["metric"], "requests");
        assert_eq!(own["prices"][0]["credits_per_unit"], "1.5");
        assert_eq!(own["pending"][0]["sync_error"], "lago 500");
        assert_eq!(
            group(&rows[0], "nyxid_managed_master")["reason"],
            "lane_not_configured"
        );

        // Legacy-only: every class resolves identically to the cached plan rate.
        let legacy = &rows[1]["effective"]["classes"];
        // Grouped by lane (platform key, own-key classes, no credential);
        // every group charges the same cached plan rate.
        assert_eq!(legacy.as_array().unwrap().len(), 3);
        for group in legacy.as_array().unwrap() {
            assert_eq!(group["reason"], "legacy_platform_billable");
            assert_eq!(group["prices"][0]["credits_per_unit"], "1.5");
            assert_eq!(group["prices"][0]["cached_rate"]["stale"], false);
        }

        // Synced BYOK lane: the service price with its own source; the pending
        // component is free until synced.
        let own = group(&rows[2], "user_owned");
        assert_eq!(own["reason"], "lane_synced");
        assert_eq!(own["source"], "service");
        assert_eq!(own["prices"].as_array().unwrap().len(), 1);
        assert_eq!(own["prices"][0]["origin"], "lane_primary");
        assert_eq!(own["prices"][0]["credits_per_unit"], "0.25");
        assert_eq!(own["prices"][0]["source"], "service");
        assert_eq!(own["pending"][0]["lago_metric_code"], "c_byok_bytes");
        assert_eq!(own["pending"][0]["source"], "free");
        assert_eq!(group(&rows[2], "no_auth")["reason"], "no_auth");
    }

    #[tokio::test]
    async fn central_billing_put_matches_the_service_editor_write_and_audit() {
        let f = with_billing_enabled(fixture("admin_billing_central_put", false).await);
        let id = insert_service(&f, "central-put", Some(json!({}))).await;
        let billing = json!({
            "platform_charge_nyxid_credentials_only": true,
            "byok_pricing": {"metric": "requests", "credits_per_unit": "0.5",
                "components": [{"metric": "bytes", "credits_per_unit": "0.000001"}]}
        });

        let editor_audit = crate::services::audit_service::notify_on_audit_write_for_user(
            "service_updated",
            f.owner.clone(),
        );
        let (status, body) = request(
            &f.state,
            "PUT",
            &format!("/api/v1/services/{id}"),
            &f.human_token,
            Some(json!({"billing": billing})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        tokio::time::timeout(std::time::Duration::from_secs(10), editor_audit)
            .await
            .expect("editor audit")
            .unwrap();
        let via_editor = stored_billing(&f, &id).await;

        f.state
            .db
            .collection::<Document>(crate::models::downstream_service::COLLECTION_NAME)
            .update_one(doc! {"_id": &id}, doc! {"$set": {"billing": {}}})
            .await
            .unwrap();
        let central_audit = crate::services::audit_service::notify_on_audit_write_for_user(
            "service_updated",
            f.owner.clone(),
        );
        let (status, body) = request(
            &f.state,
            "PUT",
            &format!("/api/v1/admin/billing/services/{id}"),
            &f.human_token,
            Some(json!({"billing": billing})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        tokio::time::timeout(std::time::Duration::from_secs(10), central_audit)
            .await
            .expect("central audit")
            .unwrap();
        assert_eq!(stored_billing(&f, &id).await, via_editor);
        let audits = audit_rows(&f, "service_updated").await;
        assert_eq!(audits.len(), 2);
        let fields = |row: &Document| {
            row.get_document("event_data")
                .unwrap()
                .get_array("changed_fields")
                .unwrap()
                .clone()
        };
        assert_eq!(fields(&audits[0]), fields(&audits[1]));
        // The central response is the refreshed effective row.
        assert_eq!(body["id"], id);
        let own = body["effective"]["classes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|g| {
                g["credential_classes"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("user_owned"))
            })
            .unwrap()
            .clone();
        assert_eq!(own["reason"], "nyxid_credentials_only");

        // Non-billing fields are refused before any write.
        let (status, body) = request(
            &f.state,
            "PUT",
            &format!("/api/v1/admin/billing/services/{id}"),
            &f.human_token,
            Some(json!({"name": "renamed", "billing": billing})),
        )
        .await;
        assert!(status.is_client_error(), "{status} {body}");
        assert_eq!(audit_rows(&f, "service_updated").await.len(), 2);
    }
}
