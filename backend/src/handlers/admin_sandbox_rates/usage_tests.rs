use crate::{
    handlers::curation_tests::{fixture, request},
    services::feature_flag_service::{self as flags, FlagTarget},
};
use axum::http::StatusCode;
use mongodb::bson::{Document, doc};
use serde_json::json;

#[tokio::test]
async fn mounted_usage_routes_gate_writes_and_reads_and_return_exact_public_dtos() {
    let f = fixture("n3_admin_routes", false).await;
    let db = &f.state.db;
    let tariff_path = "/api/v1/admin/billing/cma-usage-tariffs";
    let policy_path = "/api/v1/admin/billing/cma-usage-policies";
    let prepare_path = "/api/v1/admin/billing/cma-usage-pricing/prepare";
    let scope = json!({"producer_namespace":"cma-example","owner_id":f.owner,"sandbox_id":"sbx_synthetic","source_epoch":"00000000-0000-4000-8000-000000000002"});
    let tariff = json!({"producer_namespace":"cma-example","expected_version":0,"effective_from":"2099-01-01T00:00:01Z","rates":{"sandbox_cpu_core_nanoseconds_v1":"0.000000000001"}});
    let policy = json!({"scope":scope,"expected_version":0,"effective_from":"2099-01-01T00:00:01Z","dimensions":{"cpu":"measured","memory":"free","disk":"allocated"},"legacy_coverage":{"covered_until":"2099-01-01T00:00:00Z","manifest_digest":"a".repeat(64)}});
    let reads = [
        format!("{tariff_path}?producer_namespace=cma-example"),
        format!(
            "{policy_path}?producer_namespace=cma-example&owner_id={}&sandbox_id=sbx_synthetic&source_epoch=00000000-0000-4000-8000-000000000002",
            f.owner
        ),
    ];
    let writes = [
        (prepare_path, json!({})),
        (tariff_path, tariff.clone()),
        (policy_path, policy.clone()),
    ];
    for (path, body) in &writes {
        let (status, response) =
            request(&f.state, "POST", path, &f.human_token, Some(body.clone())).await;
        assert_eq!(status, StatusCode::PAYMENT_REQUIRED, "{response}");
    }
    flags::set_platform_override(
        db,
        flags::CMA_SANDBOX_USAGE_CHARGE_FLAG_KEY,
        &FlagTarget::Global,
        true,
        &f.owner,
    )
    .await
    .unwrap();
    for (path, body) in &writes {
        let (status, response) =
            request(&f.state, "POST", path, &f.human_token, Some(body.clone())).await;
        assert_eq!(status, StatusCode::OK, "{response}");
        assert_eq!(response["shadow_only"], true);
        if *path != prepare_path {
            assert_eq!(response["effective_from"], "2099-01-01T00:01:00Z");
            assert!(response.get("_id").is_none());
        }
        if *path == tariff_path {
            assert_eq!(response["rates"][0]["credits_per_unit"], "0.000000000001");
            assert_eq!(response["rates"][0]["provider_proof"], json!(null));
        }
        if *path == policy_path {
            assert_eq!(response["legacy_covered_until"], "2099-01-01T00:00:00Z");
        }
    }
    let roles = crate::services::role_service::get_platform_role_ids(db)
        .await
        .unwrap();
    for (role, read_status) in [
        (vec![roles.operator], StatusCode::OK),
        (vec![], StatusCode::FORBIDDEN),
    ] {
        db.collection::<Document>(crate::models::user::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&f.owner},
                doc! {"$set":{"is_admin":false,"role_ids":role}},
            )
            .await
            .unwrap();
        for path in &reads {
            let (status, body) = request(&f.state, "GET", path, &f.human_token, None).await;
            assert_eq!(status, read_status, "{body}");
        }
        for (path, body) in &writes {
            let (status, response) =
                request(&f.state, "POST", path, &f.human_token, Some(body.clone())).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{response}");
        }
    }
    let mut unknown = tariff;
    unknown["activate_charging"] = json!(true);
    assert!(serde_json::from_value::<super::TariffRequest>(unknown).is_err());
    let mut unknown = policy;
    unknown["dimensions"]["uptime"] = json!("measured");
    assert!(serde_json::from_value::<super::PolicyRequest>(unknown).is_err());
}
