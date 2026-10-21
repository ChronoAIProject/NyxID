//! Safe operator diagnostics. Configuration and API checks never certify a
//! physical writer fence, independent expiry, image startup or CSI behavior.
use crate::{AppState, errors::AppResult, mw::auth::AuthUser};
use axum::{
    Json,
    extract::{Query, State},
};
use serde::{Deserialize, Serialize};

#[derive(Default, Deserialize)]
pub struct PlatformQuery {
    inventory_cursor: Option<String>,
    inventory_limit: Option<u32>,
}

#[derive(Serialize)]
pub struct InvestigationView {
    id: String,
    kind: String,
    namespace: Option<String>,
    name: Option<String>,
    resource_uid: Option<String>,
    sandbox_uid: Option<String>,
    machine_id: Option<String>,
    owner_id: Option<String>,
    operation_id: Option<String>,
    reason: String,
    state: &'static str,
    first_seen_at: Option<String>,
    last_seen_at: Option<String>,
    operator_action_due: bool,
}

#[derive(Serialize)]
pub struct InvestigationsView {
    items: Vec<InvestigationView>,
    next_cursor: Option<String>,
    evidence: &'static str,
}

impl InvestigationView {
    fn from_record(row: mongodb::bson::Document) -> Self {
        let text = |key| {
            row.get_str(key)
                .ok()
                .filter(|value| {
                    value.len() <= 253
                        && value
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.' | b'_'))
                })
                .map(str::to_owned)
        };
        let date = |key| {
            row.get_datetime(key)
                .ok()
                .map(|v| v.to_chrono().to_rfc3339())
        };
        let kind = match row.get_str("kind").unwrap_or("Sandbox") {
            "Pod" => "Pod",
            "PersistentVolumeClaim" => "PersistentVolumeClaim",
            _ => "Sandbox",
        };
        Self {
            id: text("_id").unwrap_or_default(),
            kind: kind.into(),
            namespace: text("namespace"),
            name: text("name"),
            resource_uid: text("resource_uid")
                .or_else(|| (kind == "Sandbox").then(|| text("sandbox_uid")).flatten()),
            sandbox_uid: text("sandbox_uid"),
            machine_id: text("machine_id"),
            owner_id: text("owner_id"),
            operation_id: text("operation_id"),
            reason: text("reason").unwrap_or_else(|| "legacy_sandbox_unmatched".into()),
            state: "investigation_required",
            first_seen_at: date("created_at"),
            last_seen_at: date("last_seen_at"),
            operator_action_due: row.get_datetime("created_at").ok().is_some_and(|at| {
                at.to_chrono() + chrono::Duration::hours(24) <= chrono::Utc::now()
            }),
        }
    }
}

#[derive(Serialize)]
pub struct CheckView {
    name: &'static str,
    status: &'static str,
}
#[derive(Serialize)]
pub struct PlatformView {
    configuration: crate::services::remote_machine_manager::PlatformReadiness,
    checks: Vec<CheckView>,
    controller: crate::services::remote_machine_controller_health::ControllerHealthView,
    quota: crate::services::remote_machine_quota::Inventory,
    checked_at: String,
    upstream_api: &'static str,
    pinned_controller_commit: &'static str,
    physical_acceptance: &'static str,
    investigations: InvestigationsView,
}
pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<PlatformQuery>,
) -> AppResult<Json<PlatformView>> {
    super::login_client_context::require_first_party_human(&auth)?;
    super::admin_helpers::require_admin_or_operator(
        &state,
        &auth,
        "admin.remote_machines.platform",
    )
    .await?;
    let investigations = crate::services::remote_machine_events::list_investigations(
        &state.db,
        query.inventory_cursor.as_deref(),
        query.inventory_limit,
    )
    .await?;
    let verifier = crate::services::remote_machine_kubernetes_identity::KubernetesWorkloadVerifier::from_environment().ok();
    let storage_class =
        crate::services::remote_machine_manager::verified_storage_class_from_environment()
            .await
            .ok();
    let (mut checks, controller, quota) = tokio::join!(
        async {
            match &verifier {
                Some(verifier) => verifier
                    .operational_checks()
                    .await
                    .into_iter()
                    .map(|(name, status)| CheckView {
                        name,
                        status: match status {
                            Some(true) => "passed",
                            Some(false) => "failed",
                            None => "unknown",
                        },
                    })
                    .collect::<Vec<_>>(),
                None => vec![CheckView {
                    name: "kubernetes_configuration",
                    status: "unconfigured",
                }],
            }
        },
        crate::services::remote_machine_controller_health::observe(verifier.as_ref()),
        crate::services::remote_machine_quota::observe(
            verifier.as_ref(),
            crate::services::remote_machine_quota::Operation::Create,
            storage_class.as_deref()
        )
    );
    checks.push(CheckView {
        name: "storage_class_api_validity_not_csi_readiness",
        status: if storage_class.is_some() {
            "passed"
        } else {
            "unknown"
        },
    });
    Ok(Json(PlatformView {
        configuration: crate::services::remote_machine_manager::platform_readiness(
            &state.config.base_url,
        ),
        checks,
        controller,
        quota,
        checked_at: chrono::Utc::now().to_rfc3339(),
        upstream_api: "agents.x-k8s.io/v1beta1",
        pinned_controller_commit: "86ad94178a85aa26788077a55894cdc234c5ea6a",
        physical_acceptance: "not_established_by_configuration_or_api_checks",
        investigations: InvestigationsView {
            items: investigations
                .rows
                .into_iter()
                .map(InvestigationView::from_record)
                .collect(),
            next_cursor: investigations.next_cursor,
            evidence: "read_only_metadata_requires_investigation_not_physical_cleanup_proof",
        },
    }))
}

#[cfg(test)]
mod tests {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use mongodb::bson::{Document, doc};
    use serde_json::Value;
    use tower::ServiceExt;

    #[tokio::test]
    async fn admin_platform_inventory_is_operator_visible_paginated_and_metadata_only() {
        let db =
            crate::test_utils::connect_transaction_test_database("admin_machine_inventory").await;
        crate::services::remote_machine_events::ensure_indexes(&db)
            .await
            .unwrap();
        crate::services::role_service::seed_system_roles(&db)
            .await
            .unwrap();
        let roles = crate::services::role_service::get_platform_role_ids(&db)
            .await
            .unwrap();
        let operator_id = uuid::Uuid::new_v4();
        let ordinary_id = uuid::Uuid::new_v4();
        for id in [operator_id, ordinary_id] {
            let mut user = crate::test_utils::test_user(
                &id.to_string(),
                crate::models::user::UserType::Person,
            );
            if id == operator_id {
                user.role_ids.push(roles.operator.clone());
            }
            db.collection(crate::models::user::COLLECTION_NAME)
                .insert_one(user)
                .await
                .unwrap();
        }
        let at =
            mongodb::bson::DateTime::from_chrono(chrono::Utc::now() - chrono::Duration::hours(25));
        for kind in ["Pod", "PersistentVolumeClaim"] {
            db.collection::<Document>(crate::services::remote_machine_events::ORPHAN_INVENTORY).insert_one(doc! {
                "_id":uuid::Uuid::new_v4().to_string(),"kind":kind,"namespace":"nyxid-machine","name":"retained",
                "resource_uid":uuid::Uuid::new_v4().to_string(),"sandbox_uid":uuid::Uuid::new_v4().to_string(),
                "state":"investigation_required","reason":"resource_sandbox_missing","created_at":at,"last_seen_at":at,
                "spec":{"private_environment":"never-return-this"},"annotations":{"secret":"never-return-this"},
            }).await.unwrap();
        }
        let state = crate::test_utils::test_app_state(db.clone());
        let token = |id| {
            crate::crypto::jwt::generate_access_token(
                &state.jwt_keys,
                &state.config,
                &id,
                "read write",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap()
        };
        let operator = token(operator_id);
        let ordinary = token(ordinary_id);
        let (_, private) = crate::routes::build_router_with_state(state.clone());
        let app = private.with_state(state);
        let call = |url: String, bearer: String| {
            let app = app.clone();
            async move {
                let response = app
                    .oneshot(
                        Request::builder()
                            .uri(url)
                            .header("authorization", format!("Bearer {bearer}"))
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                let status = response.status();
                let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
                (status, serde_json::from_slice::<Value>(&body).unwrap())
            }
        };
        let url = "/api/v1/admin/remote-machines/platform?inventory_limit=1";
        let (status, denied) = call(url.into(), ordinary).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
        let (status, first) = call(url.into(), operator.clone()).await;
        assert_eq!(status, StatusCode::OK, "{first}");
        assert_eq!(
            first["investigations"]["items"].as_array().unwrap().len(),
            1
        );
        assert_eq!(
            first["investigations"]["items"][0]["operator_action_due"],
            true
        );
        assert_eq!(
            first["investigations"]["items"][0]["state"],
            "investigation_required"
        );
        assert!(!first.to_string().contains("never-return-this"));
        assert!(first["investigations"]["items"][0].get("spec").is_none());
        let cursor = first["investigations"]["next_cursor"].as_str().unwrap();
        let mut pairs = url::form_urlencoded::Serializer::new(String::new());
        pairs
            .append_pair("inventory_cursor", cursor)
            .append_pair("inventory_limit", "1");
        let (status, second) = call(
            format!("/api/v1/admin/remote-machines/platform?{}", pairs.finish()),
            operator.clone(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{second}");
        assert_eq!(
            second["investigations"]["items"].as_array().unwrap().len(),
            1
        );
        assert_ne!(
            first["investigations"]["items"][0]["id"],
            second["investigations"]["items"][0]["id"]
        );
        assert!(second["investigations"]["next_cursor"].is_null());
        let (status, _) = call(
            "/api/v1/admin/remote-machines/platform?inventory_cursor=forged".into(),
            operator,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}
