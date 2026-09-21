use super::*;
use crate::models::user::UserType;
use crate::test_utils::{test_app_state, test_auth_user, test_user};

async fn connect_test_database(prefix: &str) -> Option<mongodb::Database> {
    let db = crate::test_utils::connect_test_database(prefix).await?;
    crate::services::billing::usage_rollup::ensure_indexes(&db)
        .await
        .unwrap();
    Some(db)
}
use axum::response::IntoResponse;

fn query() -> AdminUsageQuery {
    AdminUsageQuery::default()
}

#[test]
fn validates_windows_filters_and_pagination() {
    let now = Utc::now();
    let default = query().validate(now).unwrap();
    assert_eq!(
        default.window.from,
        crate::services::billing::usage_rollup::hour(now - chrono::Duration::hours(24))
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

#[tokio::test]
async fn hourly_cost_partitions_preserve_legacy_rounding_and_unknown_masking() {
    use crate::services::billing::usage_rollup::{fold_once, hour};
    let db = connect_test_database("rollup_cost_partitions")
        .await
        .expect("MongoDB required");
    let end = hour(Utc::now()) + chrono::Duration::minutes(17);
    db.collection::<Document>("billing_rate_cache")
        .insert_one(doc! { "_id": "tokens:*", "credits_per_unit_pico": 600_001_i64 })
        .await
        .unwrap();
    for (key, ack, exact, code) in [
        ("a", false, false, "tokens"),
        ("b", false, false, "tokens"),
        ("a", true, false, "tokens"),
        ("c", true, false, "missing"),
        ("d", true, true, "missing"),
    ] {
        let mut row = meter("actor", "owner", "service", 1);
        row.insert(
            "created_at",
            bson::DateTime::from_chrono(end - chrono::Duration::hours(2)),
        );
        row.insert("api_key_id", key);
        row.insert("lago_acked", ack);
        if !ack {
            row.insert("status", "dead_letter");
        }
        row.insert("wallet_id", "wallet");
        row.insert("released", true);
        row.insert("lago_metric_code", code);
        if exact {
            row.insert(
                "funding",
                doc! { "settled": true, "total_charge_micros": 17_i64 },
            );
        }
        insert(&db, row).await;
    }
    // This row stays in the current-hour raw tail. Its display group must
    // combine with the folded "a" / unacked group before truncation: two
    // 0.600001-microcredit quantities yield one microcredit, not two zeros.
    let mut tail = meter("actor", "owner", "service", 1);
    tail.insert(
        "created_at",
        bson::DateTime::from_chrono(end - chrono::Duration::seconds(1)),
    );
    tail.insert("api_key_id", "a");
    tail.insert("lago_acked", false);
    tail.insert("status", "dead_letter");
    tail.insert("wallet_id", "wallet");
    tail.insert("released", true);
    insert(&db, tail).await;
    let params = query().validate(end).unwrap();
    let oracle = aggregate(&db, summary_pipeline(&params)).await.unwrap();
    let expected = stats(&documents(&oracle[0], "totals").unwrap()[0]).unwrap();
    assert_eq!(expected.gross_cost_micros, Some(18));
    let before = get_usage(&db, query().validate(end).unwrap())
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&before.totals).unwrap(),
        serde_json::to_value(&expected).unwrap()
    );
    while fold_once(&db, end).await.unwrap() > 0 {}
    let after = get_usage(&db, query().validate(end).unwrap())
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&after.totals).unwrap(),
        serde_json::to_value(&expected).unwrap()
    );
    db.drop().await.unwrap();
}

/// Production-density seed: 31 days of summaries, with 72h17m of raw rows. Use a disposable
/// replica-set DB via NYXID_TEST_DATABASE_URL. No production data is touched.
#[tokio::test]
#[ignore = "seeds 31 days of summaries and 1.8M raw meters; checks oracle and latency budgets"]
async fn hourly_rollup_production_density_benchmark() {
    use crate::services::billing::usage_rollup::{self, fold_once, hour};
    use serde_json::json;
    use std::time::Instant;
    // Keep a failed benchmark's synthetic data for diagnosis. An explicit
    // nyxid_benchmark_* database in the existing test URI resumes that seed;
    // successful runs always drop it. Other database names are rejected.
    let uri = std::env::var("NYXID_TEST_DATABASE_URL").expect("replica-set test URI required");
    let options = mongodb::options::ClientOptions::parse(uri).await.unwrap();
    let name = options
        .default_database
        .clone()
        .unwrap_or_else(|| format!("nyxid_benchmark_{}", uuid::Uuid::new_v4().simple()));
    assert!(
        name.starts_with("nyxid_benchmark_"),
        "benchmark requires a disposable database name"
    );
    let client = mongodb::Client::with_options(options).unwrap();
    let db = client.database(&name);
    assert!(usage_rollup::supports_transactions(&db).await.unwrap());
    println!("benchmark database: {}", db.name());
    crate::db::ensure_indexes(&db).await.unwrap();
    let metadata = db.collection::<Document>("benchmark_metadata");
    let previous = metadata.find_one(doc! { "_id": "seed" }).await.unwrap();
    let end = previous
        .as_ref()
        .map(|s| s.get_datetime("end").unwrap().to_chrono())
        .unwrap_or_else(|| hour(Utc::now()) + chrono::Duration::minutes(17));
    let first = hour(end) - chrono::Duration::hours(72);
    let actors: Vec<String> = previous
        .as_ref()
        .map(|s| {
            s.get_array("actors")
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_owned())
                .collect()
        })
        .unwrap_or_else(|| (0..15).map(|_| uuid::Uuid::new_v4().to_string()).collect());
    if previous.is_none() {
        assert_eq!(
            db.collection::<Document>(COLLECTION_NAME)
                .estimated_document_count()
                .await
                .unwrap(),
            0,
            "partial seed: drop this disposable database before restarting"
        );
        let classes = [
            "user_owned",
            "agent_override_user_owned",
            "nyxid_managed_master",
            "nyxid_platform_oauth_app",
            "node_managed",
            "no_auth",
        ];
        let metrics = [
            "tokens",
            "input_tokens",
            "output_tokens",
            "cache_read_tokens",
            "images",
        ];
        for metric in metrics {
            db.collection::<Document>("billing_rate_cache")
                .insert_one(
                    doc! { "_id": format!("{metric}:*"), "credits_per_unit_pico": 600_001_i64 },
                )
                .await
                .unwrap();
        }
        let seeded = Instant::now();
        for bucket in 0..73 {
            for chunk in 0..5 {
                let mut rows = Vec::with_capacity(5_000);
                for n in 0..5_000 {
                    let i = chunk * 5_000 + n;
                    let at = first
                        + chrono::Duration::hours(bucket)
                        + chrono::Duration::milliseconds(i as i64 * 144);
                    if at >= end {
                        break;
                    }
                    let service = format!("service-{:02}", i % 17);
                    let actor = &actors[i % 15];
                    let mut row = meter(
                        actor,
                        if i % 11 == 0 {
                            &actors[(i + 1) % 15]
                        } else {
                            actor
                        },
                        &service,
                        (i % 90 + 1) as i64,
                    );
                    row.insert("created_at", bson::DateTime::from_chrono(at));
                    row.insert("credential_class", classes[i % 6]);
                    row.insert("metric", metrics[i % 5]);
                    row.insert("lago_metric_code", metrics[i % 5]);
                    row.insert("rollup_pending", true);
                    row.insert("released", true);
                    row.insert("lago_acked", true);
                    row.insert("api_key_id", format!("agent-{}", i % 3));
                    row.insert("token_breakdown", doc! { "prompt_tokens": 23_i64, "completion_tokens": 7_i64, "cached_tokens": 3_i64, "cache_creation_tokens": 1_i64 });
                    if i % 5 == 0 {
                        row.insert(
                            "transaction_id",
                            format!(
                                "{}:component:output",
                                row.get_str("transaction_id").unwrap()
                            ),
                        );
                    }
                    if i % 7 == 0 {
                        row.insert("layer", "resale");
                        row.insert(
                            "transaction_id",
                            format!("{}:resale", row.get_str("billing_request_id").unwrap()),
                        );
                    }
                    if i % 6 != 5 {
                        row.insert("wallet_id", "wallet");
                        if i % 4 != 0 {
                            row.insert("funding", doc! { "settled": true, "total_charge_micros": 123_i64, "wallet_funded_micros": 100_i64, "grant_funded_micros": 13_i64, "allowance_funded_micros": 10_i64 });
                            row.insert("lago_acked", false);
                        }
                    }
                    rows.push(row);
                }
                if !rows.is_empty() {
                    db.collection::<Document>(COLLECTION_NAME)
                        .insert_many(rows)
                        .await
                        .unwrap();
                }
            }
            if bucket % 12 == 11 {
                println!(
                    "seeded {} hours ({:.1}s)",
                    bucket + 1,
                    seeded.elapsed().as_secs_f64()
                );
            }
        }
        for i in 0..125 {
            let mut row = meter(&actors[i % 15], &actors[i % 15], "service-00", 7);
            row.insert(
                "created_at",
                bson::DateTime::from_chrono(end - chrono::Duration::seconds(1)),
            );
            row.insert("rollup_pending", true);
            insert(&db, row).await;
        }
        metadata
            .insert_one(
                doc! { "_id": "seed", "end": bson::DateTime::from_chrono(end), "actors": &actors },
            )
            .await
            .unwrap();
    }
    let fold_explain = db.run_command(doc! { "explain": { "find": COLLECTION_NAME, "filter": usage_rollup::pending_filter(usage_rollup::cutoff(end)), "sort": { "created_at": -1 }, "limit": usage_rollup::BATCH_SIZE }, "verbosity": "executionStats" }).await.unwrap();
    assert!(
        serde_json::to_string(&fold_explain)
            .unwrap()
            .contains("IXSCAN")
    );
    let start = Instant::now();
    let mut folded = db
        .collection::<Document>(COLLECTION_NAME)
        .count_documents(doc! { "rollup_pending": false })
        .await
        .unwrap() as usize;
    loop {
        let count = fold_once(&db, end).await.unwrap();
        if count == 0 {
            break;
        }
        folded += count;
        if folded.is_multiple_of(200_000) {
            println!(
                "folded {folded} rows ({:.1}s)",
                start.elapsed().as_secs_f64()
            );
        }
    }
    let tail = db
        .collection::<Document>(COLLECTION_NAME)
        .count_documents(doc! { "rollup_pending": true })
        .await
        .unwrap() as usize;
    assert_eq!(folded + tail, 1_807_209);
    assert!(
        tail < 1_000,
        "only one minute of production traffic plus 125 live rows stays raw"
    );
    let fold_seconds = previous
        .as_ref()
        .and_then(|s| s.get_f64("fold_seconds").ok())
        .unwrap_or_else(|| start.elapsed().as_secs_f64());
    metadata
        .update_one(
            doc! { "_id": "seed" },
            doc! { "$set": { "fold_seconds": fold_seconds } },
        )
        .await
        .unwrap();
    // Every generated hour repeats the same dimensions and quantities. Copy a
    // completely folded hour for days 4–31; no raw sources are needed there.
    // The oracle below independently groups a raw hour and scales additive
    // measures BEFORE pricing/rounding, never using these summaries as truth.
    let rollups = db.collection::<Document>(crate::models::usage_rollup_hourly::COLLECTION_NAME);
    let template: Vec<Document> = rollups
        .find(doc! { "hour": bson::DateTime::from_chrono(first) })
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    let summaries_per_hour = template.len();
    if !previous
        .as_ref()
        .is_some_and(|s| s.get_bool("history_seeded").unwrap_or(false))
    {
        let history_started = Instant::now();
        let mut daily_hours = BTreeMap::<DateTime<Utc>, i64>::new();
        for n in 1..=28 * 24 {
            let at = first - chrono::Duration::hours(n);
            *daily_hours.entry(usage_rollup::day(at)).or_default() += 1;
            let rows: Vec<Document> = template
                .iter()
                .map(|r| {
                    let mut r = r.clone();
                    r.insert("_id", format!("history-{n}-{}", r.get_str("_id").unwrap()));
                    r.insert("hour", bson::DateTime::from_chrono(at));
                    r
                })
                .collect();
            rollups.insert_many(rows).await.unwrap();
            if n % 168 == 0 {
                println!(
                    "seeded {n} historical hours ({:.1}s)",
                    history_started.elapsed().as_secs_f64()
                );
            }
        }
        for (day, hours) in daily_hours {
            usage_rollup::tests::seed_benchmark_day(&db, &template, day, hours).await;
        }
        metadata
            .update_one(
                doc! { "_id": "seed" },
                doc! { "$set": { "history_seeded": true } },
            )
            .await
            .unwrap();
    }
    let hourly_documents = rollups.count_documents(doc! {}).await.unwrap();
    let daily_documents = db
        .collection::<Document>(crate::models::usage_rollup_daily::COLLECTION_NAME)
        .count_documents(doc! {})
        .await
        .unwrap();
    assert_eq!(rollups.count_documents(doc! { "hour": { "$gte": bson::DateTime::from_chrono(hour(end) - chrono::Duration::days(31)), "$lt": bson::DateTime::from_chrono(hour(end)) } }).await.unwrap(), 31 * 24 * summaries_per_hour as u64);
    println!(
        "31-day population: {summaries_per_hour} summaries/full hour; production observed 38 actor×service rows across 14 users/17 services in 2h (~3 metrics, 2 classes, a few models)"
    );
    let mut measurements = Vec::new();
    let mut failures = Vec::new();
    for (days, budget_ms) in [(1, 500.0), (7, 1_000.0), (31, 2_000.0)] {
        for filter in ["none", "user", "service", "both"] {
            let make_query = || AdminUsageQuery {
                period: (days != 31).then(|| if days == 1 { "24h".into() } else { "7d".into() }),
                from: (days == 31).then(|| (hour(end) - chrono::Duration::days(days)).to_rfc3339()),
                to: (days == 31).then(|| hour(end).to_rfc3339()),
                user: matches!(filter, "user" | "both").then(|| actors[0].clone()),
                service: matches!(filter, "service" | "both").then(|| "service-00".into()),
                ..Default::default()
            };
            let params = make_query().validate(end).unwrap();
            // The oracle intentionally has a generous bound: this is exactly
            // the slow raw scan being replaced, not an endpoint SLA assertion.
            let mut oracle_pipeline = summary_pipeline(&params);
            let old_hours = (first - params.window.from).num_hours().max(0);
            if old_hours > 0 {
                let mut template_filter = meter_filter(&params, true);
                template_filter.insert("created_at", doc! { "$gte": bson::DateTime::from_chrono(first), "$lt": bson::DateTime::from_chrono(first + chrono::Duration::hours(1)) });
                let mut scale = doc! { "_id": 1 };
                let mut combine = doc! { "_id": "$_id" };
                for field in usage_rollup::MEASURES
                    .iter()
                    .filter(|f| **f != "rows_folded")
                {
                    scale.insert(
                        *field,
                        doc! { "$multiply": [format!("${field}"), old_hours] },
                    );
                    combine.insert(*field, doc! { "$sum": format!("${field}") });
                }
                oracle_pipeline.splice(3..3, [
                    doc! { "$unionWith": { "coll": COLLECTION_NAME, "pipeline": [
                        { "$match": template_filter }, meter_flags(), { "$group": meter_group(true) }, { "$project": scale },
                    ] } },
                    doc! { "$group": combine },
                ]);
            }
            let oracle: Vec<Document> = db
                .collection::<Document>(COLLECTION_NAME)
                .aggregate(oracle_pipeline)
                .max_time(Duration::from_secs(180))
                .allow_disk_use(true)
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
            let expected = documents(&oracle[0], "totals")
                .unwrap()
                .first()
                .map(stats)
                .transpose()
                .unwrap()
                .unwrap_or_default();
            let mut ms = Vec::new();
            for _ in 0..5 {
                let started = Instant::now();
                let actual = get_usage(&db, make_query().validate(end).unwrap())
                    .await
                    .unwrap();
                ms.push(started.elapsed().as_secs_f64() * 1_000.0);
                assert_eq!(
                    serde_json::to_value(&actual.totals).unwrap(),
                    serde_json::to_value(&expected).unwrap(),
                    "{days} days / {filter}"
                );
            }
            ms.sort_by(f64::total_cmp);
            let explain = db.run_command(doc! { "explain": { "aggregate": COLLECTION_NAME, "pipeline": fast_pipeline(&params, cached_rates(&db).await.unwrap(), usage_rollup::state(&db).await.unwrap().and_then(|s| s.folded_before), usage_rollup::state(&db).await.unwrap().is_some_and(|s| s.daily_ready)), "cursor": {}, "hint": usage_rollup::PENDING_INDEX }, "verbosity": "executionStats" }).await.unwrap();
            let mut docs = 0_i64;
            let mut keys = 0_i64;
            fn examined(value: &Bson, docs: &mut i64, keys: &mut i64) {
                match value {
                    Bson::Document(d) => {
                        if let Some(v) = d.get("totalDocsExamined") {
                            *docs += match v {
                                Bson::Int32(n) => i64::from(*n),
                                Bson::Int64(n) => *n,
                                _ => 0,
                            };
                        }
                        if let Some(v) = d.get("totalKeysExamined") {
                            *keys += match v {
                                Bson::Int32(n) => i64::from(*n),
                                Bson::Int64(n) => *n,
                                _ => 0,
                            };
                        }
                        // SBE repeats these totals in nested execution stages.
                        // Count each cursor's executionStats once, not its
                        // child-stage copies as additional collection reads.
                        if d.contains_key("totalDocsExamined") {
                            return;
                        }
                        for (_, value) in d {
                            examined(value, docs, keys);
                        }
                    }
                    Bson::Array(a) => {
                        for value in a {
                            examined(value, docs, keys);
                        }
                    }
                    _ => (),
                }
            }
            examined(&Bson::Document(explain.clone()), &mut docs, &mut keys);
            println!(
                "{days}d {filter}: p50 {:.1}ms, max {:.1}ms, docs {docs}, keys {keys}",
                ms[2], ms[4]
            );
            measurements.push(json!({ "days": days, "filter": filter, "from": params.window.from, "to": params.window.to, "p50_ms": ms[2], "max_ms": ms[4], "docs_examined": docs, "keys_examined": keys, "oracle_equal": true, "explain": explain }));
            // Save evidence before asserting, so failed performance runs remain
            // inspectable and can guide optimization.
            let evidence = json!({ "population_days": 31, "hourly_documents": hourly_documents, "daily_documents": daily_documents, "summaries_per_full_hour": summaries_per_hour, "direct_history_hours": 28 * 24, "production_observed": "38 actor×service rows across 14 users and 17 services in 2h; ~3 metrics, 2 classes, a few models (not an upper bound)", "rows": folded + tail, "tail_rows": tail, "effective_backfill_rows_per_second": folded as f64 / fold_seconds * 0.75, "users": 15, "services": 17, "credential_classes": 6, "metrics": 5, "fold_seconds": fold_seconds, "fold_rows_per_second": folded as f64 / fold_seconds, "fold_explain": fold_explain, "measurements": measurements });
            std::fs::write(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .unwrap()
                    .join(".claude-brief/allowance-bundles-usage-benchmark.json"),
                serde_json::to_string_pretty(&evidence).unwrap(),
            )
            .unwrap();
            // The generator has one display partition per summary. Neither
            // tier should fetch documents, including Mongo's natural actor/owner
            // OR plan (forcing one index for that OR would disable coverage).
            assert_eq!(
                docs,
                if days == 31 { 0 } else { tail as i64 },
                "summary reductions must be covered: {days}d {filter}"
            );
            if days == 1 {
                assert!(
                    docs < 5_000,
                    "24h must examine tail-scale documents, got {docs}"
                );
            }
            if ms[2] >= budget_ms {
                failures.push(format!(
                    "{days}d {filter}: {:.1}ms exceeds {budget_ms}ms",
                    ms[2]
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "latency budgets exceeded: {failures:?}"
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn persistently_active_standalone_batch_returns_unvalidated_success_within_bound() {
    use crate::models::usage_rollup_state::{COLLECTION_NAME as STATE, STATE_ID};
    use crate::services::billing::usage_rollup;
    let db = connect_test_database("usage_active_standalone")
        .await
        .unwrap();
    usage_rollup::ensure_indexes(&db).await.unwrap();
    let now = usage_rollup::hour(Utc::now()) + chrono::Duration::minutes(17);
    let mut row = meter("actor", "owner", "service", 10);
    row.insert(
        "created_at",
        bson::DateTime::from_chrono(now - chrono::Duration::minutes(2)),
    );
    insert(&db, row).await;
    usage_rollup::fold_once(&db, now).await.unwrap();
    let increment = db
        .collection::<Document>(crate::models::usage_rollup_hourly::COLLECTION_NAME)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    db.collection::<Document>(STATE).update_one(doc! { "_id": STATE_ID }, doc! { "$set": { "batch": {
        "sequence": 2_i64, "row_ids": ["in-flight"], "increments": [increment], "claimed_at": bson::DateTime::from_chrono(now),
    } } }).await.unwrap();
    let params = query().validate(now).unwrap();
    let (result, freshness) = tokio::time::timeout(
        Duration::from_secs(2),
        fast_aggregate_with_mode(&db, &params, false),
    )
    .await
    .expect("bounded retries must finish well before the request guard")
    .unwrap();
    assert!(!freshness.validated);
    assert_eq!(
        stats(&documents(&result, "totals").unwrap()[0])
            .unwrap()
            .events,
        1
    );
    if !usage_rollup::supports_transactions(&db).await.unwrap() {
        // The same regression is also run against a real standalone mongod:
        // exercise authorization, the production topology decision, enrichment,
        // and the actual HTTP handler instead of only the aggregation helper.
        crate::services::role_service::seed_system_roles(&db)
            .await
            .unwrap();
        let uid = uuid::Uuid::new_v4().to_string();
        let mut user = test_user(&uid, UserType::Person);
        user.is_admin = true;
        user.role_ids.push(
            crate::services::role_service::get_platform_role_ids(&db)
                .await
                .unwrap()
                .admin,
        );
        db.collection::<crate::models::user::User>("users")
            .insert_one(user)
            .await
            .unwrap();
        let response = tokio::time::timeout(
            Duration::from_secs(2),
            crate::handlers::admin_usage::get_usage(
                axum::extract::State(test_app_state(db.clone())),
                test_auth_user(&uid),
                crate::telemetry::TelemetryContext::default(),
                axum::extract::Query(query()),
            ),
        )
        .await
        .expect("standalone HTTP read must be bounded")
        .unwrap();
        assert!(!response.0.freshness.validated);
        assert_eq!(
            response.into_response().status(),
            axum::http::StatusCode::OK
        );
    }
    assert!(
        usage_rollup::state(&db)
            .await
            .unwrap()
            .unwrap()
            .batch
            .is_some()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn hourly_integer_cost_reduction_is_exact_below_saturation_and_clamps_overflow() {
    use crate::services::billing::usage_rollup::{fold_once, hour};
    let db = connect_test_database("usage_hourly_integer_costs")
        .await
        .unwrap();
    let now = hour(Utc::now()) + chrono::Duration::minutes(17);
    for (service, first, second, expected) in [
        ("above-double-precision", 1_i64 << 53, 1, (1_i64 << 53) + 1),
        ("near-max", i64::MAX - 17, 7, i64::MAX - 10),
        ("overflow", i64::MAX - 17, 30, i64::MAX),
    ] {
        for (hours, amount) in [(2, first), (1, second)] {
            let mut row = meter("actor", "owner", service, 1);
            row.insert(
                "created_at",
                bson::DateTime::from_chrono(now - chrono::Duration::hours(hours)),
            );
            row.insert("wallet_id", "wallet");
            row.insert("released", true);
            row.insert("lago_acked", false);
            row.insert("funding", doc! { "settled": true, "total_charge_micros": amount, "wallet_funded_micros": amount });
            insert(&db, row).await;
        }
        let oracle = aggregate(&db, summary_pipeline(&query().validate(now).unwrap()))
            .await
            .unwrap();
        while fold_once(&db, now).await.unwrap() > 0 {}
        let actual = get_usage(&db, query().validate(now).unwrap())
            .await
            .unwrap();
        let service_row = actual
            .by_service
            .iter()
            .find(|row| row.service.service_slug.as_deref() == Some(service))
            .unwrap();
        assert_eq!(service_row.usage.gross_cost_micros, Some(expected));
        assert_eq!(service_row.usage.wallet_cost_micros, Some(expected));
        assert_eq!(
            serde_json::to_value(&actual.totals).unwrap(),
            serde_json::to_value(stats(&documents(&oracle[0], "totals").unwrap()[0]).unwrap())
                .unwrap()
        );
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn daily_hourly_edges_and_tail_match_raw_summary_and_ranking() {
    use crate::services::billing::usage_rollup::{self, day, fold_once};
    let db = connect_test_database("usage_daily_parity").await.unwrap();
    usage_rollup::ensure_indexes(&db).await.unwrap();
    let end = day(Utc::now()) + chrono::Duration::hours(17) + chrono::Duration::minutes(17);
    let first = day(end) - chrono::Duration::days(8);
    let actors = [
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
    ];
    db.collection::<Document>("billing_rate_cache")
        .insert_one(doc! { "_id": "tokens:*", "credits_per_unit_pico": 600_001_i64 })
        .await
        .unwrap();
    let mut rows = vec![];
    for day in 0..9 {
        for hour in [0, 1, 6, 12, 23] {
            let at = first + chrono::Duration::days(day) + chrono::Duration::hours(hour);
            if at >= end {
                continue;
            }
            for n in 0..12 {
                let mut row = meter(
                    &actors[n % 2],
                    &actors[(n / 2) % 2],
                    if n % 3 == 0 { "other" } else { "service" },
                    1,
                );
                row.insert(
                    "created_at",
                    bson::DateTime::from_chrono(at + chrono::Duration::minutes(n as i64)),
                );
                row.insert("api_key_id", format!("key-{}", n % 3));
                row.insert("wallet_id", "wallet");
                row.insert("released", true);
                row.insert("lago_acked", n % 4 != 0);
                if n % 4 == 0 {
                    row.insert("status", "dead_letter");
                }
                if n % 3 == 0 {
                    row.insert("funding", doc! { "settled": true, "total_charge_micros": 17_i64, "wallet_funded_micros": 9_i64, "grant_funded_micros": 5_i64, "allowance_funded_micros": 3_i64 });
                }
                if n % 5 == 0 {
                    row.insert("lago_metric_code", "missing");
                }
                if n % 7 == 0 {
                    row.insert("layer", "resale");
                }
                rows.push(row);
            }
        }
    }
    let mut live = meter(&actors[0], &actors[1], "service", 7);
    live.insert(
        "created_at",
        bson::DateTime::from_chrono(end - chrono::Duration::seconds(1)),
    );
    rows.push(live);
    db.collection::<Document>(COLLECTION_NAME)
        .insert_many(rows)
        .await
        .unwrap();
    while fold_once(&db, end).await.unwrap() > 0 {}
    assert!(usage_rollup::state(&db).await.unwrap().unwrap().daily_ready);
    // Compare the complete raw reductions (not just totals), including cost
    // partitions crossing daily/hourly/live sources, custom edges and paging.
    let canonical = |rows: Vec<Document>| -> BTreeMap<String, serde_json::Value> {
        rows.into_iter()
            .map(|r| {
                (
                    serde_json::to_string(id(&r).unwrap()).unwrap(),
                    serde_json::to_value(stats(&r).unwrap()).unwrap(),
                )
            })
            .collect()
    };
    for (from, to) in [
        (first, end),
        (first + chrono::Duration::hours(6), end),
        (
            first + chrono::Duration::minutes(5),
            end - chrono::Duration::hours(25),
        ),
        (first, first + chrono::Duration::days(1)),
        (
            first + chrono::Duration::minutes(2),
            first + chrono::Duration::minutes(9),
        ),
    ] {
        for user in [None, Some(actors[0].clone())] {
            for service in [None, Some("service".to_owned())] {
                let params = AdminUsageQuery {
                    from: Some(from.to_rfc3339()),
                    to: Some(to.to_rfc3339()),
                    user: user.clone(),
                    service,
                    sort: Some("cost".into()),
                    per_page: Some(2),
                    ..Default::default()
                }
                .validate(end)
                .unwrap();
                let raw = aggregate(&db, summary_pipeline(&params)).await.unwrap();
                let (fast, freshness) = fast_aggregate(&db, &params).await.unwrap();
                assert!(freshness.validated);
                for field in ["totals", "services", "classes", "service_classes"] {
                    assert_eq!(
                        canonical(documents(&fast, field).unwrap()),
                        canonical(documents(&raw[0], field).unwrap()),
                        "{field} {from}..{to}"
                    );
                }
                let raw_rank = aggregate(&db, ranking_pipeline(&params)).await.unwrap();
                assert_eq!(
                    documents(&fast, "ranking")
                        .unwrap()
                        .iter()
                        .map(|r| id(r).unwrap())
                        .collect::<Vec<_>>(),
                    documents(&raw_rank[0], "ranking")
                        .unwrap()
                        .iter()
                        .map(|r| id(r).unwrap())
                        .collect::<Vec<_>>()
                );
                assert_eq!(
                    canonical(documents(&fast, "ranking").unwrap()),
                    canonical(documents(&raw_rank[0], "ranking").unwrap())
                );
                assert_eq!(
                    documents(&fast, "total").unwrap(),
                    documents(&raw_rank[0], "total").unwrap()
                );
            }
        }
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn hourly_and_daily_reductions_have_covering_indexes() {
    use crate::services::billing::usage_rollup::{self, day, fold_once};
    let db = connect_test_database("usage_daily_covering").await.unwrap();
    usage_rollup::ensure_indexes(&db).await.unwrap();
    let end = day(Utc::now());
    let mut row = meter("actor", "owner", "service", 1);
    row.insert(
        "created_at",
        bson::DateTime::from_chrono(end - chrono::Duration::hours(2)),
    );
    insert(&db, row).await;
    fold_once(&db, end).await.unwrap();
    for (collection, bucket) in [
        (crate::models::usage_rollup_hourly::COLLECTION_NAME, "hour"),
        (crate::models::usage_rollup_daily::COLLECTION_NAME, "day"),
    ] {
        for index in [
            "usage_rollup_reduce_window",
            "usage_rollup_reduce_actor",
            "usage_rollup_reduce_owner",
        ] {
            let mut group = doc! { "_id": "$single_display_key" };
            for field in usage_rollup::MEASURES
                .iter()
                .filter(|f| **f != "rows_folded")
            {
                group.insert(*field, doc! { "$sum": format!("${field}") });
            }
            let mut filter = doc! { bucket: { "$gte": bson::DateTime::from_chrono(end - chrono::Duration::days(1)), "$lt": bson::DateTime::from_chrono(end) }, "single_display_key": { "$ne": null } };
            match index {
                "usage_rollup_reduce_actor" => {
                    filter.insert("actor", "actor");
                }
                "usage_rollup_reduce_owner" => {
                    filter.insert("owner", "owner");
                }
                _ => (),
            }
            let explain = db.run_command(doc! { "explain": { "aggregate": collection, "pipeline": [{ "$match": filter }, { "$group": group }], "hint": index, "cursor": {} }, "verbosity": "executionStats" }).await.unwrap();
            fn covered(value: &Bson) -> bool {
                match value {
                    Bson::Document(d) if d.contains_key("totalDocsExamined") => {
                        d.get_i32("totalDocsExamined")
                            .ok()
                            .map(i64::from)
                            .or_else(|| d.get_i64("totalDocsExamined").ok())
                            == Some(0)
                    }
                    Bson::Document(d) => d.values().any(covered),
                    Bson::Array(a) => a.iter().any(covered),
                    _ => false,
                }
            }
            assert!(
                covered(&explain.clone().into()),
                "{collection} / {index}: {explain}"
            );
            assert!(serde_json::to_string(&explain).unwrap().contains("IXSCAN"));
        }
    }
    db.drop().await.unwrap();
}
