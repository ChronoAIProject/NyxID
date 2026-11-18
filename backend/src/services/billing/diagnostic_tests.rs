use super::*;
use mongodb::bson::{self, Bson};

#[tokio::test]
async fn integrity_producers_report_their_explicit_severities() {
    let db = crate::test_utils::connect_transaction_test_database("integrity_severities").await;
    let at = bson::DateTime::from_millis(1_794_700_800_000);
    usage_comparison::TEST_NOW.scope(std::cell::Cell::new(at), async {
        db.collection::<Document>("usage_meter").create_indexes(crate::db::billing_hold_indexes()).await.unwrap();
        db.collection::<Document>("usage_meter").create_indexes(stuck_requests::indexes()).await.unwrap();
        db.collection::<Document>("security_replicas").update_one(doc! {"_id":"test-generation"}, vec![doc! {"$set":{
            "generation_id":"test-generation", "observed_at":"$$NOW", "started_at":"$$NOW"
        }}]).upsert(true).await.unwrap();
        assert!(startup_diagnostics(&db).await.unwrap().is_empty());

        db.collection::<Document>("platform_model_attempts").insert_one(doc! {"_id":"attempt", "state":"pending_review"}).await.unwrap();
        for id in ["exact-v2", "exact-v2-rollups"] {
            db.collection::<Document>("billing_migrations")
                .replace_one(doc! {"_id":id}, doc! {"_id":id, "updated_at":at})
                .upsert(true).await.unwrap();
        }
        db.collection::<Document>("billing_plan_charges").insert_one(doc! {"_id":"repair", "plan_code":"test-plan", "metric_code":"test-metric", "repair_failed_at":at}).await.unwrap();
        db.collection::<Document>("billing_rate_diagnostics").insert_one(doc! {"_id":"rates", "rejected_metrics":["test-metric"], "updated_at":at}).await.unwrap();
        db.collection::<Document>("pool_recovery_diagnostics").insert_one(doc! {"_id":"pool", "name":"pool_recovery", "failures":1_i64, "samples":[], "updated_at":at}).await.unwrap();
        db.collection::<Document>(crate::models::coordination::LEASE_COLLECTION_NAME).insert_one(doc! {"_id":"billing:reconcile", "checkpoint":{
            "version":1_i64, "failures":{"settlement_recovery":{
                "consecutive_passes":1_i64, "first_failed_at":at, "last_failed_at":at, "error_code":8000_i32
            }}
        }}).await.unwrap();
        db.collection::<Document>("billing_hold_observations").insert_many([
            doc! {"_id":"exact", "kind":"wallet_excess", "exact":true, "persistent":true, "difference":"1", "wallet_id":"test-wallet"},
            doc! {"_id":"incomplete", "kind":"grant", "exact":false, "difference":"unknown"},
        ]).await.unwrap();
        let epoch = bson::DateTime::from_millis(0);
        db.collection::<Document>("billing_wallet").insert_one(doc! {
            "_id":"test-wallet", "owner_id":"test-owner", "lago_customer_id":"test-customer",
            "plan_kind":"prepaid", "collection_state":"good", "balance_synced_at":epoch,
            "created_at":epoch, "updated_at":epoch,
            "active_topup_expiry":{
                "operation_id":"operation", "processing_token":"test-token", "lease_until":epoch,
                "amount":crate::models::credits::Credits::ZERO, "items":[],
                "dispatch_started_at":epoch, "unresolved_reported_at":epoch,
                "created_at":epoch, "updated_at":epoch
            }
        }).await.unwrap();

        use crate::models::billing_usage_comparison::{COLLECTION_NAME, CONTROL_COLLECTION, COVERAGE_ID};
        db.collection::<Document>(CONTROL_COLLECTION).insert_one(doc! {
            "_id":COVERAGE_ID, "ready":true, "generation":"test-generation",
            "continuous_since":epoch, "observed_at":at
        }).await.unwrap();
        for (status, reason) in [("drift", "difference"), ("unsupported", "subscription_unavailable"), ("coverage", "warming_full_period")] {
            db.collection::<Document>(COLLECTION_NAME).insert_one(doc! {
                "_id":status, "owner_id":"test-owner", "subscription_id":format!("test-{status}"),
                "acknowledgement_sequence":0_i64, "uncertainty_revision":0_i64, "coverage_verified":true,
                "comparison":{
                    "status":status, "reason":reason, "generation":"test-generation", "sequence":0_i64,
                    "uncertainty_revision":0_i64, "start_second":0_i64, "end_second":at.timestamp_millis()/1000 + 86400,
                    "local":{"requests":"2"}, "remote":{"requests":"1"}, "differences":{"requests":"1"},
                    "first_observed_at":at, "observed_at":at
                }
            }).await.unwrap();
        }
        let diagnostics = startup_diagnostics(&db).await.unwrap();
        let expected = [
            ("platform_model_usage_pending", DiagnosticSeverity::Action),
            ("billing_exact_cutover", DiagnosticSeverity::Action),
            ("billing_rollup_normalization", DiagnosticSeverity::Info),
            ("billing_plan_repair:repair", DiagnosticSeverity::Action),
            ("billing_invalid_rates", DiagnosticSeverity::Action),
            ("billing_pool_recovery", DiagnosticSeverity::Action),
            ("billing_reconcile_steps", DiagnosticSeverity::Action),
            ("billing_hold_conservation", DiagnosticSeverity::Action),
            ("billing_hold_conservation_incomplete", DiagnosticSeverity::Action),
            ("billing_expiry_unresolved", DiagnosticSeverity::Info),
            ("billing_usage_drift", DiagnosticSeverity::Action),
            ("billing_usage_unsupported", DiagnosticSeverity::Action),
            ("billing_usage_coverage", DiagnosticSeverity::Info),
        ];
        assert_eq!(diagnostics.len(), expected.len(), "{diagnostics:?}");
        for (code, severity) in expected {
            let diagnostic = diagnostics.iter().find(|d| d.code == code).unwrap_or_else(|| panic!("missing {code}: {diagnostics:?}"));
            assert_eq!(diagnostic.severity, severity, "{code}");
            assert!(diagnostic.technical_details.is_none(), "{code}");
        }
        db.collection::<Document>(CONTROL_COLLECTION).update_one(doc! {"_id":COVERAGE_ID}, doc! {"$set":{"ready":false}}).await.unwrap();
        let warming = usage_comparison::diagnostics(&db).await.unwrap();
        assert_eq!(warming.len(), 1);
        assert_eq!(warming[0].severity, DiagnosticSeverity::Info);
        assert_eq!(warming[0].code, "billing_usage_coverage");

        for checkpoint in [Bson::String("malformed".into()), Bson::Document(doc! {"version":99_i64})] {
            db.collection::<Document>(crate::models::coordination::LEASE_COLLECTION_NAME).update_one(doc! {"_id":"billing:reconcile"}, doc! {"$set":{"checkpoint":checkpoint}}).await.unwrap();
            let diagnostics = reconcile_status::diagnostics(&db).await;
            assert_eq!(diagnostics.len(), 1);
            assert_eq!(diagnostics[0].severity, DiagnosticSeverity::Action);
        }
    }).await;
    db.drop().await.unwrap();
}
