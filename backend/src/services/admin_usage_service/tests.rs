use super::*;
use crate::models::user::UserType;
use crate::test_utils::{connect_test_database, test_app_state, test_auth_user, test_user};
use axum::response::IntoResponse;

fn query() -> AdminUsageQuery {
    AdminUsageQuery::default()
}

#[test]
fn validates_windows_filters_and_pagination() {
    let now = Utc::now();
    let default = query().validate(now).unwrap();
    assert_eq!(
        default.window.to - default.window.from,
        chrono::Duration::hours(24)
    );
    for (from, to, valid) in [
        ("2026-01-01T00:00:00Z", "2026-02-01T00:00:00Z", true),
        ("2026-01-01T00:00:00Z", "2026-02-01T00:00:01Z", false),
        ("2026-01-01T00:00:00Z", "2026-01-01T00:00:00Z", false),
        ("2026-02-01T00:00:00Z", "2026-01-01T00:00:00Z", false),
        ("bad", "2026-02-01T00:00:00Z", false),
    ] {
        assert_eq!(
            AdminUsageQuery {
                from: Some(from.into()),
                to: Some(to.into()),
                ..query()
            }
            .validate(now)
            .is_ok(),
            valid
        );
    }
    for invalid in [
        AdminUsageQuery {
            from: Some(now.to_rfc3339()),
            ..query()
        },
        AdminUsageQuery {
            period: Some("all".into()),
            ..query()
        },
        AdminUsageQuery {
            user: Some(".*".into()),
            ..query()
        },
        AdminUsageQuery {
            sort: Some("$where".into()),
            ..query()
        },
        AdminUsageQuery {
            metric: Some("$where".into()),
            ..query()
        },
        AdminUsageQuery {
            page: Some(0),
            ..query()
        },
        AdminUsageQuery {
            page: Some(u64::MAX),
            ..query()
        },
        AdminUsageQuery {
            per_page: Some(101),
            ..query()
        },
    ] {
        assert!(invalid.validate(now).is_err());
    }
}

fn meter(actor: &str, owner: &str, service: &str, quantity: i64) -> Document {
    let request = uuid::Uuid::new_v4().to_string();
    doc! {
        "_id": uuid::Uuid::new_v4().to_string(), "billing_request_id": &request,
        "transaction_id": format!("{request}:platform"), "layer": "platform",
        "actor_user_id": actor, "billing_owner_id": owner,
        "service_id": service, "service_slug": service,
        "metric": "tokens", "lago_metric_code": "tokens", "credential_class": "user_owned",
        "quantity": quantity, "status": "finalized", "forwarded": true,
        "created_at": bson::DateTime::from_chrono(Utc::now() - chrono::Duration::minutes(1)),
    }
}

async fn insert(db: &mongodb::Database, row: Document) {
    db.collection::<Document>(COLLECTION_NAME)
        .insert_one(row)
        .await
        .unwrap();
}
async fn read(db: &mongodb::Database, query: AdminUsageQuery) -> AdminUsageResponse {
    get_usage(db, query.validate(Utc::now()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn identities_without_display_names_use_email() {
    let db = connect_test_database("admin_usage_identity_email")
        .await
        .expect("MongoDB required");
    for display_name in [
        Bson::Null,
        Bson::String(String::new()),
        Bson::String("  ".into()),
    ] {
        let actor = uuid::Uuid::new_v4().to_string();
        let email = format!("{actor}@example.test");
        db.collection::<Document>("users")
            .insert_one(doc! { "_id": &actor, "display_name": display_name, "email": &email })
            .await
            .unwrap();
        insert(&db, meter(&actor, &actor, "example", 1)).await;
        let result = read(
            &db,
            AdminUsageQuery {
                user: Some(actor),
                ..query()
            },
        )
        .await;
        assert_eq!(result.ranking[0].user.display_name, email);
        assert_eq!(result.selected_user.unwrap().display_name, email);
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn deduplicates_components_and_resale_and_attributes_org_usage() {
    let db = connect_test_database("admin_usage_dedupe")
        .await
        .expect("MongoDB required");
    let alice = uuid::Uuid::new_v4().to_string();
    let org = uuid::Uuid::new_v4().to_string();
    let bob = uuid::Uuid::new_v4().to_string();
    for (id, name, kind) in [(&alice, "Alice", "person"), (&org, "Team", "org")] {
        db.collection::<Document>("users").insert_one(doc! { "_id": id, "display_name": name, "email": format!("{name}@example.test"), "user_type": kind }).await.unwrap();
    }
    db.collection::<Document>("downstream_services")
        .insert_one(doc! { "_id": "llm", "slug": "llm", "name": "Language model" })
        .await
        .unwrap();
    let mut primary = meter(&alice, &org, "llm", 100);
    primary.insert("metric", "input_tokens");
    primary.insert("wallet_id", "wallet");
    primary.insert("token_breakdown", doc! { "prompt_tokens": 100, "completion_tokens": 20, "cached_tokens": 25, "cache_creation_tokens": 5 });
    primary.insert("funding", doc! { "total_charge_micros": 100, "wallet_funded_micros": 30, "grant_funded_micros": 20, "allowance_funded_micros": 50 });
    insert(&db, primary.clone()).await;
    for (suffix, layer, metric, quantity) in [
        (":component:output", "platform", "output_tokens", 20),
        (":resale", "resale", "tokens", 120),
    ] {
        let mut row = primary.clone();
        row.insert("_id", uuid::Uuid::new_v4().to_string());
        row.insert(
            "transaction_id",
            format!("{}{suffix}", primary.get_str("transaction_id").unwrap()),
        );
        row.insert("layer", layer);
        row.insert("metric", metric);
        row.insert("quantity", quantity);
        // Historical rows deliberately duplicate the provider breakdown.
        insert(&db, row).await;
    }
    let mut free = meter(&bob, &bob, "images", 2);
    free.insert("metric", "images");
    free.insert("credential_class", "nyxid_managed_master");
    insert(&db, free).await;
    for (status, forwarded, quantity) in [
        ("dead_letter", false, Some(900)),
        ("forwarded", true, Some(900)),
        ("finalized", true, None),
    ] {
        let mut excluded = meter(&alice, &org, "excluded", 900);
        excluded.insert("status", status);
        excluded.insert("forwarded", forwarded);
        excluded.insert("quantity", bson::to_bson(&quantity).unwrap());
        insert(&db, excluded).await;
    }
    let mut forwarded = meter(&bob, &bob, "images", 1);
    forwarded.insert("status", "dead_letter");
    forwarded.insert("metric", "images");
    insert(&db, forwarded).await;
    let mut outside = meter(&alice, &org, "excluded", 900);
    outside.insert(
        "created_at",
        bson::DateTime::from_chrono(Utc::now() - chrono::Duration::days(2)),
    );
    insert(&db, outside).await;
    let result = read(&db, query()).await;
    assert_eq!((result.totals.requests, result.totals.events), (3, 5));
    assert_eq!(
        (result.totals.unique_users, result.totals.unique_services),
        (2, 2)
    );
    assert_eq!(result.totals.quantities["input_tokens"], 100);
    assert_eq!(result.totals.quantities["output_tokens"], 20);
    assert_eq!(result.totals.quantities["images"], 3);
    assert_eq!(result.totals.total_tokens, 120);
    assert_eq!(result.totals.cached_tokens, 25);
    assert_eq!(result.totals.cache_creation_tokens, 5);
    assert_eq!(result.totals.gross_cost_micros, Some(300));
    assert_eq!(result.totals.wallet_cost_micros, Some(90));
    assert_eq!(result.totals.grant_cost_micros, Some(60));
    assert_eq!(result.totals.allowance_cost_micros, Some(150));
    let alice_row = result
        .ranking
        .iter()
        .find(|row| row.user.id == alice)
        .unwrap();
    assert_eq!(alice_row.user.display_name, "Alice");
    assert_eq!(
        alice_row.billing_owner.as_ref().unwrap().display_name,
        "Team"
    );
    assert_eq!(alice_row.service.service_name, "Language model");
    assert_eq!(
        result
            .ranking
            .iter()
            .find(|row| row.user.id == bob)
            .unwrap()
            .user
            .display_name,
        "Unknown user"
    );
    for user in [&alice, &org] {
        let filtered = read(
            &db,
            AdminUsageQuery {
                user: Some(user.clone()),
                ..query()
            },
        )
        .await;
        assert_eq!(filtered.totals.requests, 1);
        assert_eq!(filtered.totals.events, 3);
        assert_eq!(filtered.selected_user.unwrap().id, *user);
    }
    let filtered = read(
        &db,
        AdminUsageQuery {
            service: Some("images".into()),
            ..query()
        },
    )
    .await;
    assert_eq!(filtered.by_service.len(), 1);
    assert_eq!(
        filtered.services.len(),
        2,
        "service options survive selecting a service"
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn exact_and_legacy_costs_use_model_rates_and_preserve_unknowns() {
    let db = connect_test_database("admin_usage_costs")
        .await
        .expect("MongoDB required");
    let actor = uuid::Uuid::new_v4().to_string();
    for (id, micros, pico) in [
        ("tokens:*", 99, None),
        ("tokens:model", 0, Some(250_001_i64)),
    ] {
        db.collection::<Document>("billing_rate_cache").insert_one(doc! { "_id": id, "credits_per_unit_micros": micros, "credits_per_unit_pico": pico }).await.unwrap();
    }
    for _ in 0..2 {
        let mut row = meter(&actor, &actor, "precise", 2);
        row.insert("model", "model");
        row.insert("wallet_id", "wallet");
        insert(&db, row).await;
    }
    let mut funded = meter(&actor, &actor, "funded", 10);
    funded.insert("wallet_id", "wallet");
    funded.insert("funding", doc! { "grant_consumptions": [{ "amount_micros": 100 }], "allowance_consumptions": [{ "quantity": 2 }] });
    insert(&db, funded).await;
    // Exact and legacy in the same billing group; missing rate poisons that
    // group's gross/wallet/allowance, while grant remains known (get_usage).
    for exact in [false, true] {
        let mut unknown = meter(&actor, &actor, "unknown", 10);
        unknown.insert("lago_metric_code", "missing");
        unknown.insert("wallet_id", "wallet");
        if exact {
            unknown.insert(
                "funding",
                doc! { "total_charge_micros": 500, "grant_funded_micros": 30 },
            );
        }
        insert(&db, unknown).await;
    }
    let result = read(&db, query()).await;
    let precise = result
        .by_service
        .iter()
        .find(|s| s.service.service_slug.as_deref() == Some("precise"))
        .unwrap();
    assert_eq!(
        precise.usage.gross_cost_micros,
        Some(1),
        "truncate after group multiplication"
    );
    let funded = result
        .by_service
        .iter()
        .find(|s| s.service.service_slug.as_deref() == Some("funded"))
        .unwrap();
    assert_eq!(funded.usage.gross_cost_micros, Some(990));
    assert_eq!(funded.usage.wallet_cost_micros, Some(692));
    assert_eq!(funded.usage.allowance_cost_micros, Some(198));
    let unknown = result
        .by_service
        .iter()
        .find(|s| s.service.service_slug.as_deref() == Some("unknown"))
        .unwrap();
    assert_eq!(unknown.usage.gross_cost_micros, None);
    assert_eq!(unknown.usage.grant_cost_micros, Some(30));
    assert_eq!(
        result.totals.gross_cost_micros,
        Some(991),
        "skip unknown groups, retain known costs"
    );
    assert_eq!(result.totals.unknown_cost_events, 1);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn ranking_is_paged_in_mongo_and_all_metrics_and_lanes_remain_visible() {
    let db = connect_test_database("admin_usage_rank")
        .await
        .expect("MongoDB required");
    let alice = uuid::Uuid::new_v4().to_string();
    let bob = uuid::Uuid::new_v4().to_string();
    for (i, metric) in BillingMetric::ALL.iter().enumerate() {
        let mut row = meter(&alice, &alice, "all", (i + 1) as i64);
        row.insert("metric", metric.as_str());
        row.insert(
            "credential_class",
            [
                "user_owned",
                "nyxid_managed_master",
                "agent_override_user_owned",
                "node_managed",
                "no_auth",
                "nyxid_platform_oauth_app",
            ][i % 6],
        );
        insert(&db, row).await;
    }
    insert(&db, meter(&bob, &bob, "bob", 50)).await;
    let page1 = read(
        &db,
        AdminUsageQuery {
            sort: Some("quantity".into()),
            per_page: Some(1),
            ..query()
        },
    )
    .await;
    assert_eq!(page1.ranking[0].user.id, bob);
    assert_eq!(page1.ranking_total, 2);
    assert_eq!(page1.totals.quantities.len(), BillingMetric::ALL.len());
    assert_eq!(page1.by_credential_class.len(), 6);
    let page2 = read(
        &db,
        AdminUsageQuery {
            sort: Some("quantity".into()),
            page: Some(2),
            per_page: Some(1),
            ..query()
        },
    )
    .await;
    assert_eq!(page2.ranking[0].user.id, alice);
    let empty = read(
        &db,
        AdminUsageQuery {
            page: Some(3),
            per_page: Some(1),
            ..query()
        },
    )
    .await;
    assert!(empty.ranking.is_empty());
    assert_eq!(empty.ranking_total, 2);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn handler_allows_admin_and_operator_and_rejects_regular_users() {
    let db = connect_test_database("admin_usage_auth")
        .await
        .expect("MongoDB required");
    crate::services::role_service::seed_system_roles(&db)
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    for (admin, operator) in [(false, false), (true, false), (false, true)] {
        let uid = uuid::Uuid::new_v4().to_string();
        let mut user = test_user(&uid, UserType::Person);
        user.is_admin = admin;
        user.is_operator = operator;
        let role = if admin {
            "admin"
        } else if operator {
            "operator"
        } else {
            "user"
        };
        let roles = crate::services::role_service::get_platform_role_ids(&db)
            .await
            .unwrap();
        if role == "admin" {
            user.role_ids.push(roles.admin);
        } else if role == "operator" {
            user.role_ids.push(roles.operator);
        }
        db.collection::<crate::models::user::User>("users")
            .insert_one(user)
            .await
            .unwrap();
        let result = crate::handlers::admin_usage::get_usage(
            axum::extract::State(state.clone()),
            test_auth_user(&uid),
            crate::telemetry::TelemetryContext::default(),
            axum::extract::Query(query()),
        )
        .await;
        if admin || operator {
            assert_eq!(result.unwrap().0.totals.requests, 0);
        } else {
            assert!(matches!(result, Err(AppError::Forbidden(_))));
        }
    }
    assert_eq!(
        AppError::AdminUsageQueryTimeout.into_response().status(),
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    );
    let schema = <crate::api_docs::ApiDoc as utoipa::OpenApi>::openapi();
    assert!(schema.paths.paths.contains_key("/api/v1/admin/usage"));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn platform_cost_totals_saturate_without_float_rounding() {
    let db = connect_test_database("admin_usage_money_max")
        .await
        .expect("MongoDB required");
    let actor = uuid::Uuid::new_v4().to_string();
    for _ in 0..2 {
        let mut row = meter(&actor, &actor, "large", 1);
        row.insert("wallet_id", "wallet");
        row.insert(
            "funding",
            doc! { "total_charge_micros": i64::MAX, "wallet_funded_micros": i64::MAX },
        );
        insert(&db, row).await;
    }
    let result = read(&db, query()).await;
    assert_eq!(result.totals.gross_cost_micros, Some(i64::MAX));
    assert_eq!(result.totals.wallet_cost_micros, Some(i64::MAX));
    db.drop().await.unwrap();
}

/// Reproducible performance evidence against a disposable database only.
/// Run with the same NYXID_TEST_DATABASE_URL as the full backend suite:
/// cargo test -p nyxid --bin nyxid-server admin_query_execution_stats -- --ignored --nocapture
#[tokio::test]
#[ignore = "seeds 200k meters and 500k audit entries; writes explain evidence"]
async fn admin_query_execution_stats() {
    use crate::services::admin_audit_service::{self as audit, AdminAuditLogListParams};
    use mongodb::{IndexModel, options::IndexOptions};
    use serde_json::json;
    use std::time::Instant;
    let db = connect_test_database("admin_query_explain")
        .await
        .expect("MongoDB required");
    let now = Utc::now();
    let audit_collection = db.collection::<Document>("audit_log");
    let usage_collection = db.collection::<Document>(COLLECTION_NAME);
    // Main 0.27.0 index baseline, limited to the measured collections. The
    // production ensure_indexes call below must preserve EVERY baseline index.
    for keys in [
        doc! { "user_id": 1, "created_at": -1 },
        doc! { "event_type": 1, "created_at": -1 },
        doc! { "api_key_id": 1, "created_at": -1 },
    ] {
        audit_collection
            .create_index(IndexModel::builder().keys(keys).build())
            .await
            .unwrap();
    }
    audit_collection
        .create_index(
            IndexModel::builder()
                .keys(doc! { "seq": 1 })
                .options(
                    IndexOptions::builder()
                        .name("audit_log_seq_unique".to_string())
                        .unique(true)
                        .partial_filter_expression(doc! { "seq": { "$exists": true } })
                        .build(),
                )
                .build(),
        )
        .await
        .unwrap();
    for key in [
        "event_type",
        "api_key_name",
        "api_key_id",
        "user_id",
        "ip_address",
        "user_agent",
        "event_data.response_status",
        "created_at",
    ] {
        let keys = if key == "created_at" {
            doc! { "created_at": 1, "_id": 1 }
        } else {
            doc! { key: 1, "created_at": 1, "_id": 1 }
        };
        audit_collection
            .create_index(
                IndexModel::builder()
                    .keys(keys)
                    .options(
                        IndexOptions::builder()
                            .name(format!("audit_log_sort_{}", key.replace('.', "_")))
                            .build(),
                    )
                    .build(),
            )
            .await
            .unwrap();
    }
    for keys in [
        doc! { "billing_owner_id": 1, "created_at": -1 },
        doc! { "status": 1, "lago_acked": 1, "updated_at": 1 },
        doc! { "status": 1, "released": 1, "settlement_next_retry_at": 1, "finalized_at": 1 },
    ] {
        usage_collection
            .create_index(IndexModel::builder().keys(keys).build())
            .await
            .unwrap();
    }
    usage_collection
        .create_index(
            IndexModel::builder()
                .keys(doc! { "transaction_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await
        .unwrap();
    usage_collection
        .create_index(
            IndexModel::builder()
                .keys(doc! { "pending_resale_quantity": 1 })
                .options(IndexOptions::builder().sparse(true).build())
                .build(),
        )
        .await
        .unwrap();
    usage_collection
        .create_index(
            IndexModel::builder()
                .keys(doc! { "expires_at": 1 })
                .options(IndexOptions::builder().expire_after(Duration::ZERO).build())
                .build(),
        )
        .await
        .unwrap();
    let baseline_audit: Vec<_> = audit_collection
        .list_indexes()
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    let baseline_usage: Vec<_> = usage_collection
        .list_indexes()
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    for batch in 0..40 {
        let rows = (batch * 5000..(batch + 1) * 5000).map(|i| {
            let actor = uuid::Uuid::from_u128((i % 100 + 1) as u128).to_string();
            let mut row = meter(&actor, &actor, &format!("service-{}", (i / 100) % 25), (i % 500 + 1) as i64);
            row.insert("created_at", bson::DateTime::from_chrono(now - chrono::Duration::seconds((i as i64 * 39) % (90 * 86400))));
            row.insert("status", match i % 10 { 0 => "forwarded", 1 | 2 => "dead_letter", _ => "finalized" });
            row.insert("forwarded", i % 10 != 2);
            row.insert("metric", BillingMetric::ALL[(i / 2500) % BillingMetric::ALL.len()].as_str());
            row.insert("token_breakdown", doc! { "prompt_tokens": 100, "completion_tokens": 20, "cached_tokens": 30, "cache_creation_tokens": 5 });
            if i % 5 == 0 {
                row.insert("transaction_id", format!("{}:component:output", row.get_str("transaction_id").unwrap()));
            }
            row
        }).collect::<Vec<_>>();
        usage_collection.insert_many(rows).await.unwrap();
    }
    for batch in 0..100 {
        let rows = (batch * 5000..(batch + 1) * 5000).map(|i| doc! {
            "_id": format!("audit-{i:08}"), "event_type": format!("event-{:03}", i % 120),
            "user_id": format!("user-{}", i % 100), "api_key_id": format!("key-{}", i % 50),
            "api_key_name": format!("Agent {}", i % 50), "ip_address": format!("192.0.2.{}", i % 250),
            "user_agent": if i % 211 == 0 { "benchmark-needle" } else { "benchmark-browser" },
            "event_data": { "response_status": if i % 10 == 0 { 500 } else { 200 } },
            "created_at": bson::DateTime::from_chrono(now - chrono::Duration::seconds((i as i64 * 17) % (90 * 86400))),
        }).collect::<Vec<_>>();
        audit_collection.insert_many(rows).await.unwrap();
    }
    let mut evidence = json!({ "dataset": { "usage_rows": 200000, "usage_users": 100, "usage_services": 25, "audit_rows": 500000 }, "measurements": [] });
    async fn explain(db: &mongodb::Database, command: Document) -> serde_json::Value {
        let document = db
            .run_command(doc! { "explain": command, "verbosity": "executionStats" })
            .await
            .unwrap();
        let json = serde_json::to_value(document).unwrap();
        let cursor = json
            .get("stages")
            .and_then(|stages| stages.get(0))
            .and_then(|stage| stage.get("$cursor"))
            .unwrap_or(&json);
        let stats = &cursor["executionStats"];
        serde_json::json!({ "docs_examined": stats["totalDocsExamined"], "keys_examined": stats["totalKeysExamined"], "execution_ms": stats["executionTimeMillis"], "winning_plan": cursor["queryPlanner"]["winningPlan"] })
    }
    fn record(evidence: &mut serde_json::Value, phase: &str, name: &str, value: serde_json::Value) {
        println!(
            "{phase} {name}: docs={} keys={} ms={}",
            value["docs_examined"], value["keys_examined"], value["execution_ms"]
        );
        evidence["measurements"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({ "phase": phase, "query": name, "stats": value }));
    }
    let from = (now - chrono::Duration::days(1))
        .format("%Y-%m-%d")
        .to_string();
    let to = now.format("%Y-%m-%d").to_string();
    for phase in ["before", "after"] {
        if phase == "after" {
            crate::db::ensure_indexes(&db).await.unwrap();
            let audit_after: Vec<_> = audit_collection
                .list_indexes()
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
            let usage_after: Vec<_> = usage_collection
                .list_indexes()
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(&baseline_audit).unwrap(),
                serde_json::to_value(&audit_after).unwrap(),
                "audit indexes unchanged"
            );
            for old in &baseline_usage {
                assert!(
                    usage_after
                        .iter()
                        .any(|new| serde_json::to_value(new).unwrap()
                            == serde_json::to_value(old).unwrap())
                );
            }
        }
        for name in [
            "default",
            "event_type",
            "search",
            "date",
            "search_date",
            "status",
        ] {
            let params = AdminAuditLogListParams {
                page: 1,
                per_page: 50,
                search: matches!(name, "search" | "search_date").then_some("benchmark-needle"),
                search_filters: None,
                custom_filters: None,
                event_type: (name == "event_type").then_some("event-007"),
                status: (name == "status").then_some("5xx"),
                actor: None,
                user_id: None,
                api_key_id: None,
                created_dates: None,
                created_from: matches!(name, "date" | "search_date").then_some(from.as_str()),
                created_to: matches!(name, "date" | "search_date").then_some(to.as_str()),
                sort: "-created_at",
            };
            let filter = audit::admin_audit_filter(&params).unwrap();
            let count_pipeline = vec![
                doc! { "$match": filter.clone() },
                doc! { "$group": { "_id": 1, "n": { "$sum": 1 } } },
            ];
            let mut count =
                doc! { "aggregate": "audit_log", "pipeline": count_pipeline, "cursor": {} };
            if phase == "after" && filter.is_empty() {
                // estimated_document_count issues a count command without a
                // query, allowing MongoDB's RECORD_STORE_FAST_COUNT plan.
                count = doc! { "count": "audit_log" };
            }
            if phase == "after"
                && let Some(hint) = audit::admin_count_hint(&params, &filter)
            {
                count.insert("hint", bson::to_bson(&hint).unwrap());
            }
            record(
                &mut evidence,
                phase,
                &format!("audit.{name}.count"),
                explain(&db, count).await,
            );
            let mut find = doc! { "find": "audit_log", "filter": filter.clone(), "sort": audit::admin_audit_sort(params.sort).unwrap(), "limit": 50 };
            if phase == "after"
                && let Some(hint) = audit::admin_page_hint(&params, &filter)
            {
                find.insert("hint", bson::to_bson(&hint).unwrap());
            }
            record(
                &mut evidence,
                phase,
                &format!("audit.{name}.find"),
                explain(&db, find).await,
            );
            // Exercise the production path and compare its exact count/order
            // against the original queries, beyond inspecting explain plans.
            if phase == "after" {
                let expected_count = audit_collection
                    .count_documents(filter.clone())
                    .await
                    .unwrap();
                let expected_rows: Vec<Document> = audit_collection
                    .find(filter)
                    .sort(audit::admin_audit_sort(params.sort).unwrap())
                    .limit(50)
                    .await
                    .unwrap()
                    .try_collect()
                    .await
                    .unwrap();
                let (actual_rows, actual_count) = audit::list_entries(&db, params).await.unwrap();
                assert_eq!(actual_count, expected_count);
                assert_eq!(
                    actual_rows
                        .iter()
                        .map(|row| row.id.as_str())
                        .collect::<Vec<_>>(),
                    expected_rows
                        .iter()
                        .map(|row| row.get_str("_id").unwrap())
                        .collect::<Vec<_>>()
                );
            }
        }
        let params = query().validate(now).unwrap();
        for (name, pipeline) in [
            ("summary", summary_pipeline(&params)),
            ("ranking", ranking_pipeline(&params)),
        ] {
            let value = explain(&db, doc! { "aggregate": COLLECTION_NAME, "pipeline": pipeline, "cursor": {}, "allowDiskUse": true }).await;
            if phase == "after" {
                assert!(value["docs_examined"].as_i64().unwrap() < 5000);
                assert!(value["winning_plan"].to_string().contains("IXSCAN"));
                assert!(!value["winning_plan"].to_string().contains("COLLSCAN"));
            }
            record(&mut evidence, phase, &format!("usage.{name}"), value);
        }
    }
    for sort in audit::ADMIN_SORT_OPTIONS {
        let value = explain(&db, doc! { "find": "audit_log", "filter": {}, "sort": audit::admin_audit_sort(sort).unwrap(), "limit": 50 }).await;
        assert_eq!(value["docs_examined"].as_i64(), Some(50));
        assert!(value["winning_plan"].to_string().contains("IXSCAN"));
        record(&mut evidence, "sort_coverage", sort, value);
    }
    for (label, q) in [
        (
            "30d",
            AdminUsageQuery {
                period: Some("30d".into()),
                ..query()
            },
        ),
        (
            "actor",
            AdminUsageQuery {
                user: Some(uuid::Uuid::from_u128(3).to_string()),
                ..query()
            },
        ),
        (
            "service",
            AdminUsageQuery {
                service: Some("service-3".into()),
                ..query()
            },
        ),
    ] {
        let value = explain(&db, doc! { "aggregate": COLLECTION_NAME, "pipeline": summary_pipeline(&q.validate(now).unwrap()), "cursor": {} }).await;
        assert!(value["winning_plan"].to_string().contains("IXSCAN"));
        record(&mut evidence, "window_filters", label, value);
    }
    // Add a competing date-only candidate on the disposable database and force
    // each candidate for an apples-to-apples comparison. Production keeps only
    // the status/date index; no existing index is dropped here or in production.
    usage_collection
        .create_index(
            IndexModel::builder()
                .keys(doc! { "created_at": -1 })
                .options(
                    IndexOptions::builder()
                        .name("benchmark_date_candidate".to_string())
                        .build(),
                )
                .build(),
        )
        .await
        .unwrap();
    for hint in ["benchmark_date_candidate", "usage_meter_admin_window"] {
        record(&mut evidence, "candidate", hint, explain(&db, doc! { "aggregate": COLLECTION_NAME, "pipeline": summary_pipeline(&query().validate(now).unwrap()), "cursor": {}, "hint": hint }).await);
    }
    let cache = audit::EventTypeCache::default();
    let start = Instant::now();
    let cold = cache.get(&db).await.unwrap();
    let cold_ms = start.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    let warm = cache.get(&db).await.unwrap();
    let warm_ms = start.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(cold, warm);
    evidence["event_type_cache"] =
        json!({ "cold_ms": cold_ms, "warm_ms": warm_ms, "types": cold.len(), "ttl_seconds": 30 });
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(".claude-brief/admin-query-explain.json");
    std::fs::write(path, serde_json::to_string_pretty(&evidence).unwrap()).unwrap();
    db.drop().await.unwrap();
}
