use super::*;
use crate::services::assistant_invocation_service as invocations;
use crate::services::assistant_service_execution as execution;

async fn tracked_fixture() -> (
    AsyncFixture,
    crate::models::assistant_invocation::AssistantInvocation,
) {
    let mut a = AsyncFixture::new().await;
    finish(&a, &a.f.row, "ready for owner submission").await;
    invocations::ensure_indexes(&a.f.state.db).await.unwrap();
    let key = uuid::Uuid::new_v4().to_string();
    let admission = invocations::admit(
        &a.f.state.db,
        &a.f.owner,
        &key,
        "fixture request fingerprint",
        &user_start(&a.f.row.id, "run bounded work"),
        &a.f.state.encryption_keys,
    )
    .await
    .unwrap();
    a.f.row = admission.claim.unwrap().conversation;
    a.f.chat = acks::for_key(
        &a.f.state.db,
        &a.f.owner,
        Some(&a.f.row.credential_api_key_id),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        a.f.chat.invocation_id.as_deref(),
        Some(admission.invocation.id.as_str())
    );
    set_access_mode(&a.f, crate::models::assistant_settings::AccessMode::Nyxmode).await;
    (a, admission.invocation)
}

async fn hold_operation(
    a: &AsyncFixture,
    watch: &AsyncServiceOperation,
    name: &str,
) -> crate::errors::AppResult<(
    execution::Admission,
    crate::services::proxy_service::ProxyTarget,
)> {
    let chat = acks::for_key(&a.f.state.db, &a.f.owner, Some(&watch.api_key_id))
        .await
        .unwrap()
        .unwrap();
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
    let endpoint = service.endpoints.iter().find(|e| e.name == name).unwrap();
    let authority =
        execution::AsyncContinuation::new(watch, endpoint, a.f.state.node_ws_manager.clone())
            .unwrap();
    let admission = authority
        .admit(
            &a.f.state.db,
            &chat,
            &a.service,
            &target,
            &json!({"method":"GET","path":"/executions/op-1"}),
        )
        .await?
        .admission
        .unwrap();
    Ok((admission, target))
}

#[tokio::test]
async fn invocation_async_final_egress_rechecks_root_and_original_delivery_before_sweep() {
    for mutation in ["cancel", "delete", "terminal", "missing", "delivery"] {
        let (a, root) = Box::pin(tracked_fixture()).await;
        let submitted = a.submit().await;
        assert_eq!(
            submitted.delivery.invocation_id.as_deref(),
            Some(root.id.as_str())
        );
        finish(&a, &a.f.row, "submitted").await;
        a.due().await;
        let watch = watches::claim(&a.f.state.db).await.unwrap().unwrap();
        let (admission, target) = hold_operation(&a, &watch, &watch.contract.status_operation)
            .await
            .unwrap();
        admission
            .revalidate(&a.f.state.db, &a.service, Some(&target))
            .await
            .unwrap();
        let roots =
            a.f.state
                .db
                .collection::<bson::Document>(crate::models::assistant_invocation::COLLECTION_NAME);
        match mutation {
            "cancel" | "delete" => {
                roots
                    .update_one(
                        doc! {"_id":&root.id},
                        doc! {"$set":{
                            "cancel_requested":true,"conversation_deleted":mutation == "delete",
                        }},
                    )
                    .await
                    .unwrap();
            }
            "terminal" => {
                roots
                    .update_one(
                        doc! {"_id":&root.id},
                        doc! {"$set":{"terminal_status":"completed"}},
                    )
                    .await
                    .unwrap();
            }
            "missing" => {
                roots.delete_one(doc! {"_id":&root.id}).await.unwrap();
            }
            "delivery" => {
                a.f.state
                    .db
                    .collection::<bson::Document>(WATCHES)
                    .update_one(
                        doc! {"_id":&watch.id},
                        doc! {"$set":{"delivery.invocation_id":uuid::Uuid::new_v4().to_string()}},
                    )
                    .await
                    .unwrap();
            }
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
            a.calls.lock().unwrap().as_slice(),
            &["/executions"],
            "only original submission dispatched: {mutation}"
        );
    }
}

#[tokio::test]
async fn invocation_async_cancel_after_settlement_keeps_only_one_cancel_and_status_cleanup() {
    let (a, root) = Box::pin(tracked_fixture()).await;
    let submitted = a.submit().await;
    finish(&a, &a.f.row, "submitted").await;
    a.f.state.db.collection::<bson::Document>(WATCHES).update_one(
        doc! {"_id":&submitted.id},
        doc! {"$set":{"deadline":bson::DateTime::from_chrono(chrono::Utc::now()-chrono::Duration::seconds(1))}},
    ).await.unwrap();
    Box::pin(invocations::cancel(
        &a.f.state,
        &a.f.owner,
        &root.idempotency_key,
    ))
    .await
    .unwrap();
    a.due().await;
    let watch = watches::claim(&a.f.state.db).await.unwrap().unwrap();
    assert_eq!(watch.state, "cancelling");
    assert!(
        hold_operation(&a, &watch, &watch.contract.status_operation)
            .await
            .is_err(),
        "cleanup needs its durable attempt marker"
    );
    // Return this unattempted lease to the worker. The worker durably records
    // one cancel, then performs the declared terminal-status read over HTTP.
    watches::defer(&a.f.state.db, &watch).await.unwrap();
    a.due().await;
    Box::pin(async_operations::sweep(&a.f.state)).await.unwrap();
    let stored =
        a.f.state
            .db
            .collection::<AsyncServiceOperation>(WATCHES)
            .find_one(doc! {"_id":&submitted.id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(stored.state, "cancelled");
    let calls = a.calls.lock().unwrap().clone();
    assert_eq!(
        calls
            .iter()
            .filter(|path| path.ends_with("/cancel"))
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|path| path.as_str() == "/executions/op-1")
            .count(),
        1
    );
    assert!(!calls.iter().any(|path| path.ends_with("/result")));
    a.due().await;
    Box::pin(async_operations::sweep(&a.f.state)).await.unwrap();
    assert_eq!(*a.calls.lock().unwrap(), calls, "cancel is never replayed");
}

#[tokio::test]
async fn invocation_async_cleanup_capability_cannot_fetch_results_after_cancel() {
    let (a, root) = Box::pin(tracked_fixture()).await;
    a.submit().await;
    finish(&a, &a.f.row, "submitted").await;
    Box::pin(invocations::cancel(
        &a.f.state,
        &a.f.owner,
        &root.idempotency_key,
    ))
    .await
    .unwrap();
    a.due().await;
    let watch = watches::claim(&a.f.state.db).await.unwrap().unwrap();
    assert!(
        watches::begin_tracked_cancellation(&a.f.state.db, &watch)
            .await
            .unwrap()
    );
    assert!(
        hold_operation(&a, &watch, &watch.contract.result_operation)
            .await
            .is_err()
    );
    for operation in [
        watch.contract.status_operation.as_str(),
        watch.contract.cancel_operation.as_deref().unwrap(),
    ] {
        let (admission, target) = hold_operation(&a, &watch, operation).await.unwrap();
        admission
            .revalidate(&a.f.state.db, &a.service, Some(&target))
            .await
            .unwrap();
    }
    assert_eq!(a.calls.lock().unwrap().as_slice(), &["/executions"]);
}

#[tokio::test]
async fn invocation_cleanup_deadline_exception_does_not_extend_ordinary_status_or_result() {
    let a = AsyncFixture::new().await;
    let submitted = a.submit().await;
    assert!(submitted.delivery.invocation_id.is_none());
    a.f.state.db.collection::<bson::Document>(WATCHES).update_one(
        doc! {"_id":&submitted.id},
        doc! {"$set":{
            "state":"cancelling", "poll_at":bson::DateTime::from_millis(0),
            "deadline":bson::DateTime::from_chrono(chrono::Utc::now()-chrono::Duration::seconds(1)),
        }},
    ).await.unwrap();
    let watch = watches::claim(&a.f.state.db).await.unwrap().unwrap();
    for operation in [
        &watch.contract.status_operation,
        &watch.contract.result_operation,
    ] {
        assert!(
            hold_operation(&a, &watch, operation).await.is_err(),
            "ordinary {operation} must remain expired"
        );
    }
    // Ordinary cancellation retains main's preexisting deadline exception.
    assert!(
        hold_operation(
            &a,
            &watch,
            watch.contract.cancel_operation.as_deref().unwrap()
        )
        .await
        .is_ok()
    );
    assert_eq!(a.calls.lock().unwrap().as_slice(), &["/executions"]);
}
