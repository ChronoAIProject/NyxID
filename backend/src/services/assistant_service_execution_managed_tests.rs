//! Actual HTTP/MCP managed paths retain final authority and exact funding proof.
use super::*;
use crate::models::{
    billing_wallet::BillingWallet,
    credits::Credits,
    downstream_service::{PlatformKeyAudience, PlatformKeyConfig},
    managed_execution::{ManagedExecution, ManagedExecutionState},
    service_billing::{
        BillingMetric, LanePriceComponent, LanePricing, ManagedExecutionContract,
        PricingSyncStatus, ServiceBilling,
    },
    usage_meter::UsageMeterRow,
};
use crate::services::{mcp_service, telnyx_platform_service as telnyx};
use std::sync::Arc;
use tokio::sync::Notify;

fn request() -> Value {
    json!({"model":"policy-managed-model","messages":[{"role":"user","content":"Hello"}],"max_tokens":20,"stream":false})
}
fn response(valid: bool) -> String {
    let mut body = json!({"id":"completion_policy_test","model":"policy-managed-model",
        "choices":[{"message":{"role":"assistant","content":"Hello"}}],
        "usage":{"prompt_tokens":50,"completion_tokens":5,"total_tokens":55,"prompt_tokens_details":{"cached_tokens":10}}});
    if !valid {
        body["usage"]
            .as_object_mut()
            .unwrap()
            .remove("prompt_tokens_details");
    }
    let body = body.to_string();
    format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}
fn billing() -> ServiceBilling {
    ServiceBilling {
        platform_billable: true,
        managed_execution: Some(ManagedExecutionContract {
            model: "policy-managed-model".into(),
            max_input_tokens: 100,
            max_output_tokens: 20,
            max_request_bytes: 4096,
            max_response_bytes: 4096,
            tariff_version: "policy-test-v1".into(),
            tariff_source: "https://developers.telnyx.com/docs/inference".into(),
            tariff_verified_at: Utc::now() - Duration::minutes(1),
        }),
        platform_key_pricing: Some(LanePricing {
            metric: BillingMetric::InputTokens,
            credits_per_unit: "0.000000000002".into(),
            lago_metric_code: "policy_managed_input".into(),
            sync_status: PricingSyncStatus::Synced,
            sync_error: None,
            operations: vec![],
            components: vec![
                LanePriceComponent {
                    metric: BillingMetric::OutputTokens,
                    credits_per_unit: "0.000000000003".into(),
                    lago_metric_code: "policy_managed_output".into(),
                    sync_status: PricingSyncStatus::Synced,
                    sync_error: None,
                },
                LanePriceComponent {
                    metric: BillingMetric::CacheReadTokens,
                    credits_per_unit: "0.000000000001".into(),
                    lago_metric_code: "policy_managed_cache".into(),
                    sync_status: PricingSyncStatus::Synced,
                    sync_error: None,
                },
            ],
        }),
        ..Default::default()
    }
}
async fn fixture(trailing: bool) -> (Fixture, String, String) {
    let mut f = orchestrator_fixture("managed_policy_routes").await;
    mode(&f, AccessMode::Autopilot).await;
    crate::db::ensure_indexes(&f.state.db).await.unwrap();
    crate::services::billing::ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
        crate::services::billing::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
    ));
    f.state.billing = Arc::new(crate::billing_integration_tests::billing_service(
        &f.state.db,
        Arc::new(crate::billing_integration_tests::FakeLago::default()),
        0,
    ));
    let now = bson::DateTime::now();
    f.state
        .db
        .collection::<Document>("billing_migrations")
        .update_one(doc! {"_id":"exact-v2"}, doc! {"$set":{"completed_at":now}})
        .upsert(true)
        .await
        .unwrap();
    let wallet: BillingWallet = bson::from_document(doc! {
        "_id":Uuid::new_v4().to_string(),"owner_id":&f.owner,"lago_customer_id":"customer","lago_subscription_id":"subscription","lago_wallet_id":"wallet",
        "plan_kind":"prepaid","balance_credits":Credits::from_pico(10000).unwrap(),"reserved_credits":Credits::ZERO,"collection_state":"good",
        "balance_synced_at":now,"created_at":now,"updated_at":now,
    }).unwrap();
    f.state
        .db
        .collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
        .insert_one(wallet)
        .await
        .unwrap();
    for (code, pico) in [
        ("policy_managed_input", 2),
        ("policy_managed_output", 3),
        ("policy_managed_cache", 1),
    ] {
        f.state
            .db
            .collection("billing_rate_cache")
            .insert_one(crate::models::billing_rate_cache::BillingRateCache {
                id: format!("{code}:*"),
                lago_metric_code: code.into(),
                model: None,
                credits_per_unit_micros: 0,
                credits_per_unit_pico: Some(pico),
                synced_at: Utc::now(),
                retired_at: None,
            })
            .await
            .unwrap();
    }
    let mut service = test_utils::test_auto_connected_catalog_service();
    service.slug = "managed-policy".into();
    service.base_url = if trailing {
        "https://api.telnyx.com/v2/"
    } else {
        "https://api.telnyx.com/v2"
    }
    .into();
    service.auth_method = "bearer".into();
    service.service_category = "internal".into();
    service.platform_key = Some(PlatformKeyConfig {
        enabled: true,
        audience: PlatformKeyAudience::Restricted,
        allowed_owner_ids: vec![f.owner.clone()],
    });
    service.credential_encrypted = f
        .state
        .encryption_keys
        .encrypt(Uuid::new_v4().to_string().as_bytes())
        .await
        .unwrap();
    service.billing = Some(billing());
    telnyx::validate_catalog(&service).unwrap();
    let catalog = service.id.clone();
    f.state
        .db
        .collection("downstream_services")
        .insert_one(service)
        .await
        .unwrap();
    let mut endpoint = test_utils::test_service_endpoint(
        &catalog,
        "complete",
        "POST",
        &format!("/{}", telnyx::CHAT_PATH),
    );
    endpoint.request_body_required = true;
    endpoint.request_content_type = Some("application/json".into());
    endpoint.request_body_schema = Some(json!({"type":"object","properties":{
        "model":{"type":"string"},"messages":{"type":"array","items":{"type":"object"}},
        "max_tokens":{"type":"integer"},"stream":{"type":"boolean"}},
        "required":["model","messages","max_tokens"]}));
    f.state
        .db
        .collection("service_endpoints")
        .insert_one(endpoint)
        .await
        .unwrap();
    let id = connection(&f, &f.owner, "managed-policy", Some(&catalog)).await;
    f.state.db.collection::<Document>("user_services").update_one(doc! {"_id":&id},
        doc! {"$set":{"credential_binding":"platform","auth_method":"bearer","auth_key_name":"Authorization"}}).await.unwrap();
    super::compatibility::grant(&f, &id).await;
    f.auth.allowed_service_ids.push(id.clone());
    choose(&f, &id).await;
    (f, id, catalog)
}
// Keep the resource owner distinct from the acting person and billing wallet.
async fn org_connection(f: &Fixture, id: &str, catalog: &str) {
    let owner = org(f).await;
    let row = crate::services::user_service_service::find_user_service_by_id(&f.state.db, id)
        .await
        .unwrap()
        .unwrap();
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(doc! {"_id":&id}, doc! {"$set":{"user_id":&owner}})
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("user_endpoints")
        .update_one(
            doc! {"_id":row.endpoint_id},
            doc! {"$set":{"user_id":&owner}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("downstream_services")
        .update_one(
            doc! {"_id":&catalog},
            doc! {"$set":{"platform_key.allowed_owner_ids":[&owner]}},
        )
        .await
        .unwrap();
}

async fn call(f: &Fixture, id: &str, mcp: bool) -> crate::errors::AppResult<u16> {
    if !mcp {
        use axum::{
            body::{Body, to_bytes},
            extract::{Path, State},
            http::Request,
        };
        let mut auth = f.auth.clone();
        auth.assistant_chat = f.auth.api_key_id.as_ref().map(|_| Arc::new(f.chat.clone()));
        let mut request = Request::builder()
            .method("POST")
            .uri(format!(
                "/api/v1/proxy/s/managed-policy/{}",
                telnyx::CHAT_PATH
            ))
            .header("content-type", "application/json")
            .body(Body::from(request().to_string()))
            .unwrap();
        request.extensions_mut().insert(
            crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                crate::services::billing::BillingIngress::Proxy,
            ),
        );
        let response = crate::handlers::proxy::proxy_request_by_slug(
            State(f.state.clone()),
            auth,
            Default::default(),
            Path(("managed-policy".into(), telnyx::CHAT_PATH.into())),
            request,
        )
        .await?;
        let status = response.status().as_u16();
        to_bytes(response.into_body(), 8192).await.unwrap();
        return Ok(status);
    }
    let services =
        mcp_service::load_user_tools_all(&f.state.db, &f.state.node_ws_manager, &f.owner).await?;
    let service = services
        .iter()
        .find(|s| s.service_id == id)
        .expect("managed connection is published");
    let endpoint = service
        .endpoints
        .iter()
        .find(|e| e.name == "complete")
        .unwrap();
    let prepared = mcp_service::prepare_proxy_tool_call(service, endpoint, &request())?;
    let context = mcp_service::McpExecContext {
        assistant_chat: f.auth.api_key_id.as_ref().map(|_| &f.chat),
        response_body_limit: None,
        actor_user_id: None,
        caller_token: None,
        delegation_restrictions: Default::default(),
        attribution: None,
        org_agent_access: None,
        agent_owner: None,
        operation_scopes: None,
        api_key_id: f.auth.api_key_id.as_deref(),
        allow_all_nodes: true,
        allowed_node_ids: &[],
    };
    let response = mcp_service::execute_tool_response(
        &f.state.http_client,
        &f.state.db,
        &f.state.encryption_keys,
        &f.state.node_ws_manager,
        &f.state.billing,
        &f.owner,
        &f.owner,
        service,
        endpoint,
        prepared,
        &f.state.jwt_keys,
        &f.state.config,
        &f.state.connection_expiry_notifier,
        &f.state.token_exchange_cache,
        &f.state.cloud_response_cache,
        &context,
        crate::services::billing::route_inventory::internal_node_dispatch_permit(),
    )
    .await?;
    Ok(response.status)
}
async fn execution(f: &Fixture) -> ManagedExecution {
    f.state
        .db
        .collection::<ManagedExecution>(crate::models::managed_execution::COLLECTION_NAME)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap()
}
async fn wallet(f: &Fixture) -> BillingWallet {
    f.state
        .db
        .collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
        .find_one(doc! {"owner_id":&f.owner})
        .await
        .unwrap()
        .unwrap()
}
async fn receipt_count(f: &Fixture) -> u64 {
    f.state
        .db
        .collection::<Document>("assistant_acknowledgements")
        .count_documents(
            doc! {"conversation_id":&f.row.id,"decided_by":"access_mode","status":"allowed"},
        )
        .await
        .unwrap()
}
fn pause() -> telnyx::Pause {
    telnyx::Pause {
        entered: Arc::new(Notify::new()),
        resume: Arc::new(Notify::new()),
    }
}

#[tokio::test]
async fn actual_managed_http_and_mcp_refuse_stop_after_funding_without_vendor_send() {
    refused(false, "stop").await;
}
#[tokio::test]
async fn actual_managed_http_and_mcp_final_sender_refuses_stop_and_releases_forwarded_holds() {
    refused(true, "stop").await;
}
#[tokio::test]
async fn actual_managed_http_and_mcp_final_sender_refuses_catalog_revocation() {
    refused(true, "catalog").await;
}
async fn refused(admitted: bool, drift: &str) {
    for mcp in [false, true] {
        let (f, id, catalog) = fixture(false).await;
        let mut provider = nyxid_service_adapters::test_support::scripted_fixture(
            "api.telnyx.com",
            vec![response(true)],
        )
        .await;
        let pause = pause();
        let hooks = telnyx::TestHooks {
            client: provider
                .builder
                .take()
                .unwrap()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            funded: (!admitted).then(|| pause.clone()),
            admitted: admitted.then(|| pause.clone()),
            completed: None,
        };
        let pending = telnyx::TEST_HOOKS.scope(hooks, call(&f, &id, mcp));
        tokio::pin!(pending);
        tokio::select! { result=&mut pending => panic!("request ended before authority pause: {result:?}"), ()=pause.entered.notified()=>{} }
        let row = execution(&f).await;
        assert_eq!(
            row.state,
            if admitted {
                ManagedExecutionState::DispatchAdmitted
            } else {
                ManagedExecutionState::Funded
            }
        );
        assert!(wallet(&f).await.reserved_credits > Credits::ZERO);
        if drift == "stop" {
            f.state
                .db
                .collection::<Document>("assistant_conversations")
                .update_one(
                    doc! {"_id":&f.row.id},
                    doc! {"$set":{"active_turn.stop_requested":true}},
                )
                .await
                .unwrap();
        } else {
            f.state
                .db
                .collection::<Document>("downstream_services")
                .update_one(
                    doc! {"_id":catalog},
                    doc! {"$set":{"platform_key.enabled":false}},
                )
                .await
                .unwrap();
        }
        pause.resume.notify_one();
        let error = pending
            .await
            .expect_err("revoked authority must refuse dispatch");
        assert!(
            !matches!(error, AppError::ManagedExecutionPending(_)),
            "proven final refusal must retain its definite rejection classification"
        );
        assert_eq!(execution(&f).await.state, ManagedExecutionState::Released);
        assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
        assert_eq!(wallet(&f).await.pending_lago_debits, Credits::ZERO);
        assert_eq!(
            f.state
                .db
                .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
                .count_documents(doc! {"released":true,"forwarded":false,"status":"failed"})
                .await
                .unwrap(),
            3
        );
        assert_eq!(receipt_count(&f).await, 0);
        assert!(matches!(
            provider.requests.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
    }
}

#[tokio::test]
async fn actual_managed_http_and_mcp_success_settles_once_and_records_one_receipt() {
    for mcp in [false, true] {
        let (f, id, catalog) = fixture(!mcp).await;
        if mcp {
            // The direct executor's organization policy floor yields a real
            // automatic service-action receipt for this ordinary POST.
            org_connection(&f, &id, &catalog).await;
            f.state
                .db
                .collection::<Document>(COLLECTION)
                .delete_many(
                    doc! {"kind":"selection","conversation_id":&f.row.id,"connection_id":&id},
                )
                .await
                .unwrap();
            choose(&f, &id).await;
        } else {
            // Catalog-backed HTTP resolves approval policy under the catalog ID.
            super::receipts::require_approval(&f, &catalog).await;
        }
        let mut provider = nyxid_service_adapters::test_support::scripted_fixture(
            "api.telnyx.com",
            vec![response(true)],
        )
        .await;
        let hooks = telnyx::TestHooks {
            client: provider
                .builder
                .take()
                .unwrap()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            funded: None,
            admitted: None,
            completed: None,
        };
        assert_eq!(
            telnyx::TEST_HOOKS
                .scope(hooks, call(&f, &id, mcp))
                .await
                .unwrap(),
            200,
            "mcp={mcp}"
        );
        assert_eq!(execution(&f).await.state, ManagedExecutionState::Settled);
        assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
        assert_eq!(
            wallet(&f).await.pending_lago_debits,
            Credits::from_pico(105).unwrap()
        );
        assert_eq!(receipt_count(&f).await, 1, "mcp={mcp}");
        let receipt = f
            .state
            .db
            .collection::<Document>("assistant_acknowledgements")
            .find_one(doc! {"conversation_id":&f.row.id,"decided_by":"access_mode"})
            .await
            .unwrap()
            .unwrap();
        // Direct MCP execution chooses its service-action receipt; the HTTP
        // approval handler chooses the existing approval reason instead.
        assert_eq!(
            receipt.get_str("tool_name").unwrap(),
            if mcp {
                "nyxid.service_execution"
            } else {
                "service_approval"
            },
            "mcp={mcp}"
        );
        assert_eq!(
            receipt
                .get_document("confirmation")
                .unwrap()
                .get_str("reason")
                .unwrap(),
            if mcp { "policy" } else { "approval" },
            "mcp={mcp}"
        );
        let sent = provider.requests.try_recv().unwrap();
        assert!(sent.starts_with(b"POST /v2/ai/openai/chat/completions HTTP/1.1\r\n"));
        assert!(matches!(
            provider.requests.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
    }
}

#[tokio::test]
async fn actual_managed_http_and_mcp_incomplete_usage_retains_liability_without_success_receipt() {
    for mcp in [false, true] {
        let (f, id, _) = fixture(false).await;
        let mut provider = nyxid_service_adapters::test_support::scripted_fixture(
            "api.telnyx.com",
            vec![response(false)],
        )
        .await;
        let hooks = telnyx::TestHooks {
            client: provider
                .builder
                .take()
                .unwrap()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            funded: None,
            admitted: None,
            completed: None,
        };
        assert!(matches!(
            telnyx::TEST_HOOKS.scope(hooks, call(&f, &id, mcp)).await,
            Err(AppError::ManagedExecutionPending(_))
        ));
        assert_eq!(execution(&f).await.state, ManagedExecutionState::Pending);
        assert!(wallet(&f).await.reserved_credits > Credits::ZERO);
        assert_eq!(wallet(&f).await.pending_lago_debits, Credits::ZERO);
        assert_eq!(receipt_count(&f).await, 0);
        assert!(provider.requests.try_recv().is_ok());
        assert_eq!(
            f.state
                .db
                .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
                .count_documents(doc! {"forwarded":true,"released":false})
                .await
                .unwrap(),
            3
        );
    }
}

#[tokio::test]
async fn actual_managed_exchange_finishes_accounting_after_caller_disconnect_and_stop() {
    for mcp in [false, true] {
        let (f, id, _) = fixture(false).await;
        let (mut provider, entered, release) =
            nyxid_service_adapters::test_support::scripted_fixture_paused(
                "api.telnyx.com",
                vec![response(true)],
            )
            .await;
        let completed = Arc::new(Notify::new());
        let hooks = telnyx::TestHooks {
            client: provider
                .builder
                .take()
                .unwrap()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            funded: None,
            admitted: None,
            completed: Some(completed.clone()),
        };
        {
            let pending = telnyx::TEST_HOOKS.scope(hooks, call(&f, &id, mcp));
            tokio::pin!(pending);
            tokio::select! { result=&mut pending=>panic!("response before provider release: {result:?}"),()=entered.notified()=>{} }
            assert_eq!(
                execution(&f).await.state,
                ManagedExecutionState::DispatchAdmitted
            );
            assert!(wallet(&f).await.reserved_credits > Credits::ZERO);
            // Dropping the caller here must not cancel a dispatched operation.
        }
        f.state
            .db
            .collection::<Document>("assistant_conversations")
            .update_one(
                doc! {"_id":&f.row.id},
                doc! {"$set":{"active_turn.stop_requested":true}},
            )
            .await
            .unwrap();
        release.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(10), completed.notified())
            .await
            .unwrap();
        assert_eq!(execution(&f).await.state, ManagedExecutionState::Settled);
        assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
        assert_eq!(
            wallet(&f).await.pending_lago_debits,
            Credits::from_pico(105).unwrap()
        );
        assert!(provider.requests.try_recv().is_ok());
        assert!(matches!(
            provider.requests.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
    }
}

#[tokio::test]
async fn actual_managed_nonassistant_org_connection_keeps_resource_acl_and_personal_billing() {
    for mcp in [false, true] {
        let (mut f, id, catalog) = fixture(false).await;
        org_connection(&f, &id, &catalog).await;
        f.auth = test_utils::test_auth_user(&f.owner);
        let mut provider = nyxid_service_adapters::test_support::scripted_fixture(
            "api.telnyx.com",
            vec![response(true)],
        )
        .await;
        let hooks = telnyx::TestHooks {
            client: provider
                .builder
                .take()
                .unwrap()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            funded: None,
            admitted: None,
            completed: None,
        };
        assert_eq!(
            telnyx::TEST_HOOKS
                .scope(hooks, call(&f, &id, mcp))
                .await
                .unwrap(),
            200
        );
        let execution = execution(&f).await;
        assert_eq!(execution.state, ManagedExecutionState::Settled);
        assert_eq!(execution.billing_owner_id, f.owner);
        assert_eq!(
            wallet(&f).await.pending_lago_debits,
            Credits::from_pico(105).unwrap()
        );
        assert_eq!(receipt_count(&f).await, 0);
        assert!(provider.requests.try_recv().is_ok());
    }
}

#[tokio::test]
async fn actual_managed_catalog_only_call_keeps_final_turn_and_scope_fences() {
    for drift in ["stop", "scope"] {
        let (mut f, id, catalog) = fixture(false).await;
        f.state
            .db
            .collection::<Document>("user_services")
            .delete_one(doc! {"_id":id})
            .await
            .unwrap();
        f.state
            .db
            .collection::<Document>(COLLECTION)
            .delete_many(doc! {"kind":"selection"})
            .await
            .unwrap();
        f.state
            .db
            .collection::<Document>("api_keys")
            .update_one(
                doc! {"_id":&f.chat.api_key_id},
                doc! {"$set":{"allowed_service_ids":[],"allowed_platform_service_ids":[&catalog]}},
            )
            .await
            .unwrap();
        f.auth.allowed_service_ids = vec![catalog];
        let mut provider = nyxid_service_adapters::test_support::scripted_fixture(
            "api.telnyx.com",
            vec![response(true)],
        )
        .await;
        let pause = pause();
        let hooks = telnyx::TestHooks {
            client: provider
                .builder
                .take()
                .unwrap()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            funded: None,
            admitted: Some(pause.clone()),
            completed: None,
        };
        let pending = telnyx::TEST_HOOKS.scope(hooks, call(&f, "", false));
        tokio::pin!(pending);
        tokio::select! { result=&mut pending=>panic!("catalog call did not reach final guard: {result:?}"),()=pause.entered.notified()=>{} }
        assert_eq!(
            execution(&f).await.state,
            ManagedExecutionState::DispatchAdmitted
        );
        if drift == "stop" {
            f.state
                .db
                .collection::<Document>("assistant_conversations")
                .update_one(
                    doc! {"_id":&f.row.id},
                    doc! {"$set":{"active_turn.stop_requested":true}},
                )
                .await
                .unwrap();
        } else {
            f.state
                .db
                .collection::<Document>("api_keys")
                .update_one(
                    doc! {"_id":&f.chat.api_key_id},
                    doc! {"$set":{"allowed_platform_service_ids":[]}},
                )
                .await
                .unwrap();
        }
        pause.resume.notify_one();
        let error = pending
            .await
            .expect_err("revoked catalog authority must refuse dispatch");
        assert!(
            !matches!(error, AppError::ManagedExecutionPending(_)),
            "proven catalog refusal must retain its definite rejection classification"
        );
        assert_eq!(execution(&f).await.state, ManagedExecutionState::Released);
        assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
        assert_eq!(wallet(&f).await.pending_lago_debits, Credits::ZERO);
        assert_eq!(receipt_count(&f).await, 0);
        assert!(matches!(
            provider.requests.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
    }
}
