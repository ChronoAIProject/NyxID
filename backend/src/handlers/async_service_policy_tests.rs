use super::*;
use crate::models::{assistant_settings::AccessMode, org_membership::OrgRole, user::UserType};
use crate::services::assistant_service_execution as execution;

async fn org_watch(a: &mut AsyncFixture, bound_agent: bool) -> AsyncServiceOperation {
    let owner = uuid::Uuid::new_v4().to_string();
    a.f.state
        .db
        .collection("users")
        .insert_one(crate::test_utils::test_user(&owner, UserType::Org))
        .await
        .unwrap();
    a.f.state
        .db
        .collection("org_memberships")
        .insert_one(crate::test_utils::test_membership(
            &owner,
            &a.f.owner,
            OrgRole::Admin,
            None,
        ))
        .await
        .unwrap();
    let connection =
        crate::services::user_service_service::find_user_service_by_id(&a.f.state.db, &a.service)
            .await
            .unwrap()
            .unwrap();
    a.f.state
        .db
        .collection::<bson::Document>("user_services")
        .update_one(doc! {"_id":&a.service}, doc! {"$set":{"user_id":&owner}})
        .await
        .unwrap();
    a.f.state
        .db
        .collection::<bson::Document>("user_endpoints")
        .update_one(
            doc! {"_id":&connection.endpoint_id},
            doc! {"$set":{"user_id":&owner}},
        )
        .await
        .unwrap();
    if bound_agent {
        use crate::services::{assistant_team_service as team, feature_flag_service as flags};
        flags::set_platform_override(
            &a.f.state.db,
            "assistant:org-agents",
            &flags::FlagTarget::Global,
            true,
            &a.f.owner,
        )
        .await
        .unwrap();
        let (agent, row) = team::create_specialist_for(
            &a.f.state.db,
            &a.f.state.encryption_keys,
            &a.f.owner,
            &owner,
            team::CreateRequest {
                name: "async-org".into(),
                description: "Async org worker".into(),
                display_name: None,
                persona: None,
                targets: Default::default(),
                account_read: false,
                machines: None,
                logins: None,
                specialty: None,
                created_by: "user",
            },
        )
        .await
        .unwrap()
        .unwrap();
        team::set_grants(
            &a.f.state.db,
            &a.f.owner,
            &agent.id,
            team::GrantChange::Add(crate::models::assistant_agent::AgentGrants {
                service_ids: vec![a.service.clone()],
                ..Default::default()
            }),
        )
        .await
        .unwrap();
        let row = engine::begin_turn(
            &a.f.state.db,
            &a.f.owner,
            &user_start(&row.id, "Run bounded work"),
            &a.f.state.encryption_keys,
        )
        .await
        .unwrap();
        a.f.chat = acks::for_key(&a.f.state.db, &a.f.owner, Some(&row.credential_api_key_id))
            .await
            .unwrap()
            .unwrap();
        let key = crate::services::key_service::get_api_key(
            &a.f.state.db,
            &a.f.owner,
            &row.credential_api_key_id,
        )
        .await
        .unwrap();
        assert_eq!(
            key.assistant_agent_owner_id.as_deref(),
            Some(owner.as_str())
        );
        a.f.auth = crate::mw::auth::api_key_auth_user(&a.f.state.db, &key, None, None, None)
            .await
            .unwrap();
        a.f.row = row;
    }
    set_access_mode(&a.f, AccessMode::Nyxmode).await;
    let auth = authenticate(&a.f).await;
    let args =
        json!({"Idempotency-Key":"org-request","script":"print('hello')","language":"python"});
    let card = result(
        direct_call(
            &a.f,
            &auth,
            "sandbox__submit_execution_handler",
            args.clone(),
        )
        .await,
        true,
    )
    .await;
    assert_eq!(card["error"], "acknowledgement_required");
    assert!(
        a.calls.lock().unwrap().is_empty(),
        "org submit awaits owner consent"
    );
    let id = card["acknowledgement_id"].as_str().unwrap();
    acks::decide(&a.f.state.db, &a.f.owner, &a.f.row.id, id, true)
        .await
        .unwrap();
    let mut approved = args;
    approved["acknowledgement_id"] = json!(id);
    let response = direct_call(&a.f, &auth, "sandbox__submit_execution_handler", approved).await;
    let bytes = axum::body::to_bytes(response.into_body(), 100_000)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["result"]["isError"], false, "{body}");
    let watch =
        a.f.state
            .db
            .collection::<AsyncServiceOperation>(WATCHES)
            .find_one(doc! {"state":"waiting"})
            .await
            .unwrap()
            .unwrap();
    assert!(
        watch.execution_binding.is_some(),
        "actual approved submission snapshot persisted"
    );
    // Discard only the unsuccessful proposal watch; it cannot poll a provider.
    a.f.state
        .db
        .collection::<bson::Document>(WATCHES)
        .delete_many(doc! {"operation_id":null})
        .await
        .unwrap();
    watch
}

#[tokio::test]
async fn approved_org_async_watch_runs_when_idle_or_on_a_later_turn_without_new_consent() {
    for later_turn in [false, true] {
        let mut a = AsyncFixture::new().await;
        let watch = org_watch(&mut a, false).await;
        finish(&a, &a.f.row, "submitted").await;
        if later_turn {
            engine::begin_turn(
                &a.f.state.db,
                &a.f.owner,
                &user_start(&a.f.row.id, "unrelated work"),
                &a.f.state.encryption_keys,
            )
            .await
            .unwrap();
        }
        let count = acks::history(&a.f.state.db, &a.f.owner, &a.f.row.id)
            .await
            .unwrap()
            .len();
        a.due().await;
        Box::pin(async_operations::sweep(&a.f.state)).await.unwrap();
        let row =
            a.f.state
                .db
                .collection::<AsyncServiceOperation>(WATCHES)
                .find_one(doc! {"_id":&watch.id})
                .await
                .unwrap()
                .unwrap();
        assert_eq!(row.state, "queued");
        {
            let calls = a.calls.lock().unwrap();
            assert_eq!(
                calls
                    .iter()
                    .filter(|p| p.as_str() == "/executions/op-1")
                    .count(),
                1
            );
            assert_eq!(calls.iter().filter(|p| p.ends_with("/result")).count(), 1);
        }
        assert_eq!(
            acks::history(&a.f.state.db, &a.f.owner, &a.f.row.id)
                .await
                .unwrap()
                .len(),
            count
        );
    }
}

#[tokio::test]
async fn org_owned_specialist_continues_approved_result_and_cancel_after_its_turn() {
    for cancel in [false, true] {
        let mut a = AsyncFixture::new().await;
        let watch = org_watch(&mut a, true).await;
        finish(&a, &a.f.row, "submitted").await;
        let cards = acks::history(&a.f.state.db, &a.f.owner, &a.f.row.id)
            .await
            .unwrap()
            .len();
        if cancel {
            async_operations::cancel_conversation(&a.f.state, &a.f.owner, &a.f.row.id)
                .await
                .unwrap();
        } else {
            a.due().await;
            Box::pin(async_operations::sweep(&a.f.state)).await.unwrap();
        }
        let fresh =
            a.f.state
                .db
                .collection::<AsyncServiceOperation>(WATCHES)
                .find_one(doc! {"_id":&watch.id})
                .await
                .unwrap()
                .unwrap();
        assert_eq!(fresh.state, if cancel { "cancelled" } else { "queued" });
        assert!(
            a.calls
                .lock()
                .unwrap()
                .iter()
                .any(|path| path.ends_with(if cancel { "/cancel" } else { "/result" }))
        );
        assert_eq!(
            acks::history(&a.f.state.db, &a.f.owner, &a.f.row.id)
                .await
                .unwrap()
                .len(),
            cards
        );
    }
}

#[tokio::test]
async fn org_owned_specialist_executes_exact_granted_ordinary_mcp_and_proxy_connections() {
    let mut a = AsyncFixture::new().await;
    org_watch(&mut a, true).await;
    set_access_mode(&a.f, AccessMode::Autopilot).await;
    let auth = authenticate(&a.f).await;
    let value = result(
        direct_call(
            &a.f,
            &auth,
            "sandbox__get_execution_handler",
            json!({"operation_id":"ordinary-mcp"}),
        )
        .await,
        false,
    )
    .await;
    assert_eq!(value["status"], "succeeded", "{value}");

    let credential = credentials::load_for_conversation(
        &a.f.state.db,
        &a.f.state.encryption_keys,
        &a.f.owner,
        &a.f.row.id,
    )
    .await
    .unwrap()
    .unwrap();
    let (_, router) = crate::routes::build_router_with_state(a.f.state.clone());
    use tower::ServiceExt;
    let response = router
        .with_state(a.f.state.clone())
        .oneshot(
            axum::http::Request::builder()
                .uri("/api/v1/proxy/s/sandbox/executions/ordinary-proxy")
                .header(
                    "authorization",
                    format!("Bearer {}", credential.raw_key.as_str()),
                )
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 100_000)
        .await
        .unwrap();
    assert_eq!(
        status,
        axum::http::StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["status"], "succeeded");
    let calls = a.calls.lock().unwrap();
    assert!(calls.iter().any(|path| path == "/executions/ordinary-mcp"));
    assert!(
        calls
            .iter()
            .any(|path| path == "/executions/ordinary-proxy")
    );
}

async fn revoke_service_scope(a: &AsyncFixture, key: &str) {
    a.f.state
        .db
        .collection::<bson::Document>("api_keys")
        .update_one(
            doc! {"_id":key},
            doc! {"$set":{
                "allow_all_services":false, "allow_auto_connected_services":false,
                "allowed_service_ids":[],
            }},
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn ordinary_mcp_stale_handler_auth_cannot_admit_a_revoked_personal_service() {
    let a = AsyncFixture::new().await;
    let stale = authenticate(&a.f).await;
    revoke_service_scope(&a, &a.f.row.credential_api_key_id).await;
    let response = direct_call(
        &a.f,
        &stale,
        "sandbox__get_execution_handler",
        json!({"operation_id":"revoked"}),
    )
    .await;
    let bytes = axum::body::to_bytes(response.into_body(), 100_000)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["result"]["isError"], true, "{body}");
    assert!(
        a.calls.lock().unwrap().is_empty(),
        "revoked scope must prevent egress"
    );
}

#[tokio::test]
async fn held_async_continuation_rechecks_lease_key_membership_and_execution_generation() {
    for mutation in [
        "lease",
        "key",
        "membership",
        "config",
        "contract",
        "scope",
        "service_scope",
        "legacy_scope_before_admit",
    ] {
        let mut a = AsyncFixture::new().await;
        let original = org_watch(&mut a, false).await;
        if mutation == "legacy_scope_before_admit" {
            a.f.state
                .db
                .collection::<bson::Document>(WATCHES)
                .update_one(
                    doc! {"_id":&original.id},
                    doc! {"$unset":{
                        "authority_version":"", "execution_binding":"", "contract_binding":"",
                    }},
                )
                .await
                .unwrap();
        }
        finish(&a, &a.f.row, "submitted").await;
        a.due().await;
        let watch = watches::claim(&a.f.state.db).await.unwrap().unwrap();
        assert_eq!(watch.id, original.id);
        let chat = acks::for_key(&a.f.state.db, &a.f.owner, Some(&watch.api_key_id))
            .await
            .unwrap()
            .unwrap();
        assert!(!chat.turn_live);
        let target = crate::services::proxy_service::resolve_proxy_target_by_user_service_id(
            &a.f.state.db,
            &a.f.state.encryption_keys,
            &a.f.owner,
            &a.service,
            None,
            None,
            crate::services::proxy_service::ProxyExecutionContext::new(
                None,
                crate::mw::rate_limit::PlatformUserRateLimitPolicy::disabled(),
            ),
        )
        .await
        .unwrap()
        .unwrap()
        .target;
        let services =
            mcp_service::load_user_tools_all(&a.f.state.db, &a.f.state.node_ws_manager, &a.f.owner)
                .await
                .unwrap();
        let service = services.iter().find(|s| s.service_id == a.service).unwrap();
        let endpoint = service
            .endpoints
            .iter()
            .find(|e| e.name == watch.contract.status_operation)
            .unwrap();
        let authority =
            execution::AsyncContinuation::new(&watch, endpoint, a.f.state.node_ws_manager.clone())
                .unwrap();
        if mutation == "legacy_scope_before_admit" {
            // A stale handler snapshot cannot authorize a legacy watch after
            // the current key's exact service grant has already been removed.
            revoke_service_scope(&a, &watch.api_key_id).await;
            assert!(
                authority
                    .admit(
                        &a.f.state.db,
                        &chat,
                        &a.service,
                        &target,
                        &json!({"method":"GET","path":"/executions/op-1"})
                    )
                    .await
                    .is_err()
            );
            assert_eq!(a.calls.lock().unwrap().len(), 1);
            continue;
        }
        let decision = authority
            .admit(
                &a.f.state.db,
                &chat,
                &a.service,
                &target,
                &json!({"method":"GET","path":"/executions/op-1"}),
            )
            .await
            .unwrap();
        let admission = decision.admission.unwrap();
        admission
            .revalidate(&a.f.state.db, &a.service, Some(&target))
            .await
            .unwrap();
        match mutation {
            "lease" => {
                a.f.state
                    .db
                    .collection::<bson::Document>(WATCHES)
                    .update_one(
                        doc! {"_id":&watch.id},
                        doc! {"$set":{"lease_id":"replacement"}},
                    )
                    .await
                    .unwrap();
            }
            "key" => {
                a.f.state
                    .db
                    .collection::<bson::Document>("api_keys")
                    .update_one(
                        doc! {"_id":&watch.api_key_id},
                        doc! {"$set":{"is_active":false}},
                    )
                    .await
                    .unwrap();
            }
            "membership" => {
                a.f.state
                    .db
                    .collection::<bson::Document>("org_memberships")
                    .delete_many(doc! {"member_user_id":&a.f.owner})
                    .await
                    .unwrap();
            }
            "config" => {
                a.f.state
                    .db
                    .collection::<bson::Document>("user_services")
                    .update_one(
                        doc! {"_id":&a.service},
                        doc! {"$set":{"default_request_headers":[{"name":"x-project-id","value":"replacement"}]}},
                    )
                    .await
                    .unwrap();
            }
            "contract" => {
                let changed =
                    a.f.state
                        .db
                        .collection::<bson::Document>("service_endpoints")
                        .update_one(
                            doc! {"_id":&endpoint.endpoint_id},
                            doc! {"$set":{"path":"/replacement/{operation_id}"}},
                        )
                        .await
                        .unwrap();
                assert_eq!(changed.matched_count, 1, "mutate actual stored endpoint");
            }
            "scope" => {
                a.f.state.db.collection::<bson::Document>("api_keys").update_one(doc! {"_id":&watch.api_key_id},
                    doc! {"$set":{"assistant_operation_scopes":{&a.service:{"revision":1_i64,"catalog_service_id":bson::Bson::Null,"operations":[]}}}}).await.unwrap();
            }
            "service_scope" => revoke_service_scope(&a, &watch.api_key_id).await,
            _ => unreachable!(),
        }
        assert!(
            admission
                .revalidate(&a.f.state.db, &a.service, Some(&target))
                .await
                .is_err(),
            "{mutation}"
        );
        assert_eq!(
            a.calls.lock().unwrap().len(),
            1,
            "only approved submission reached provider"
        );
    }
}

#[tokio::test]
async fn legacy_submitted_org_watches_keep_bounded_result_and_cancel_lifecycle() {
    for cancel in [false, true] {
        let mut a = AsyncFixture::new().await;
        let watch = org_watch(&mut a, false).await;
        // Model the stored shape produced by a pre-upgrade writer after its
        // ordinary submission. New writers always retain authority_version=1.
        a.f.state.db.collection::<bson::Document>(WATCHES).update_one(doc! {"_id":&watch.id},
            doc! {"$unset":{"authority_version":"","execution_binding":"","contract_binding":""}}).await.unwrap();
        finish(&a, &a.f.row, "submitted before upgrade").await;
        let cards = acks::history(&a.f.state.db, &a.f.owner, &a.f.row.id)
            .await
            .unwrap()
            .len();
        if cancel {
            async_operations::cancel_conversation(&a.f.state, &a.f.owner, &a.f.row.id)
                .await
                .unwrap();
        } else {
            a.due().await;
            Box::pin(async_operations::sweep(&a.f.state)).await.unwrap();
        }
        let fresh =
            a.f.state
                .db
                .collection::<AsyncServiceOperation>(WATCHES)
                .find_one(doc! {"_id":&watch.id})
                .await
                .unwrap()
                .unwrap();
        assert_eq!(fresh.state, if cancel { "cancelled" } else { "queued" });
        assert!(
            a.calls
                .lock()
                .unwrap()
                .iter()
                .any(|path| path.ends_with(if cancel { "/cancel" } else { "/result" }))
        );
        assert_eq!(
            acks::history(&a.f.state.db, &a.f.owner, &a.f.row.id)
                .await
                .unwrap()
                .len(),
            cards
        );
    }
}
