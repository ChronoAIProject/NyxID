use super::*;
use crate::services::{channel_guest_access as guest, feature_flag_admin_service as flags};

pub(super) async fn set_guest_flag(state: &AppState, enabled: Option<bool>) -> AppResult<()> {
    crate::test_utils::init_test_audit_chain_hmac_key();
    let actor = guest_test_actor();
    flags::mutate(
        &state.db,
        guest::FLAG,
        flags::Mutation::Override {
            kind: crate::models::feature_flag_override::FlagTargetKind::Global,
            target_id: None,
            enabled,
        },
        &actor,
    )
    .await?;
    Ok(())
}

fn guest_test_actor() -> crate::services::audit_service::AuditActor {
    crate::services::audit_service::AuditActor {
        user_id: OWNER.into(),
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
    }
}

pub(super) async fn enable_guest_test_bot(state: &AppState, bot_id: &str) {
    crate::test_utils::init_test_audit_chain_hmac_key();
    let updated = state
        .db
        .collection::<bson::Document>("channel_bots")
        .update_one(
            doc! {"_id": bot_id},
            doc! {"$set": {"guest_access_v1": true, "guest_access_v1_revision": 1_i64}},
        )
        .await
        .unwrap();
    assert_eq!(updated.matched_count, 1);
    guest::set_pilot(
        &state.db,
        guest::PilotSelection {
            bot_ids: Some(vec![bot_id.to_owned()]),
            fleet: false,
        },
        &guest_test_actor(),
    )
    .await
    .unwrap();
    set_guest_flag(state, Some(true)).await.unwrap();
}

#[tokio::test]
async fn nyxbot_guest_runtime_rollback_admits_fresh_legacy_work_while_stale_worker_is_busy() {
    let (state, calls, server) = setup("guest_runtime_active_rollback").await;
    let (row, key) = channel(&state, "gateway").await;
    assert!(matches!(
        set_guest_flag(&state, Some(true)).await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    crate::services::security_readiness_service::ready_test_replica(&state.db).await;
    chats::set_private_chats(&state, OWNER, &row.id, "everyone")
        .await
        .unwrap();
    let binding = put_binding(
        State(state.clone()),
        Path("bnd_chats".into()),
        bearer(&key),
        Json(binding_body(&key, OWNER)),
    )
    .await;
    let status = binding.status();
    assert_eq!(status, StatusCode::OK, "{}", body_text(binding).await);
    ensure_partition(&state, &key, PARTITION).await;
    let baseline = event("Ordinary baseline question", "42", "guest-runtime-before");
    assert!(
        body_text(respond(&state, &key, &baseline, "guest_runtime_before").await)
            .await
            .contains("Here is your answer")
    );
    let ordinary = channel_conversations(&state, &row.id).await.remove(0);
    assert!(ordinary.id.starts_with("nyxa-"));

    enable_guest_test_bot(&state, &row.channel_bot_id).await;
    let guest::GuestMode::AddOn {
        activation_epoch, ..
    } = guest::select_mode(&state.db, &row.channel_bot_id)
        .await
        .unwrap()
    else {
        panic!("expected add-on mode")
    };
    let old = event("Blocked add-on request", "42", "guest-runtime-stale");
    let stale_worker = guest::test_hooks::pause_worker(&state.db);
    let blocked_model = calls.lock().await;
    let old_response = respond(&state, &key, &old, "guest_runtime_stale").await;
    assert_eq!(old_response.status(), StatusCode::OK);
    tokio::time::timeout(Duration::from_secs(30), stale_worker.wait_for_provider())
        .await
        .expect("the old guest work must reach the mock provider before disabling");
    let addon = channel_conversations(&state, &row.id)
        .await
        .into_iter()
        .find(|row| guest::is_hosted_conversation(&row.id))
        .expect("add-on conversation");
    let active = addon.active_turn.as_ref().expect("add-on model is busy");
    assert_eq!(
        active.channel_guest.as_ref().unwrap().activation_epoch,
        activation_epoch
    );
    assert_ne!(ordinary.credential_api_key_id, addon.credential_api_key_id);
    guest::require_live_request(
        &state.db,
        OWNER,
        &addon.id,
        &active.turn_id,
        &addon.credential_api_key_id,
    )
    .await
    .unwrap();

    set_guest_flag(&state, Some(false)).await.unwrap();
    assert_eq!(
        guest::select_mode(&state.db, &row.channel_bot_id)
            .await
            .unwrap(),
        guest::GuestMode::Legacy
    );
    assert!(
        guest::revalidate(&state.db, active.channel_guest.as_ref().unwrap())
            .await
            .is_err()
    );
    assert!(
        guest::require_live_request(
            &state.db,
            OWNER,
            &addon.id,
            &active.turn_id,
            &addon.credential_api_key_id,
        )
        .await
        .is_err(),
        "old authenticated worker must fail its final model dispatch check"
    );
    drop(blocked_model);
    let legacy = event("Fresh ordinary request", "42", "guest-runtime-after");
    let response = respond(&state, &key, &legacy, "guest_runtime_after").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(body_text(response).await.contains("Here is your answer"));
    let rows = channel_conversations(&state, &row.id).await;
    assert_eq!(rows.len(), 2);
    let stale = rows.iter().find(|row| row.id == addon.id).unwrap();
    assert!(
        stale.active_turn.is_some(),
        "the old turn is deliberately unsettled"
    );
    let fresh = rows.iter().find(|row| row.id == ordinary.id).unwrap();
    assert_eq!(fresh.credential_api_key_id, ordinary.credential_api_key_id);

    drop(stale_worker);
    assert!(
        !body_text(old_response)
            .await
            .contains("Here is your answer")
    );
    let replay = respond(&state, &key, &old, "guest_runtime_stale").await;
    assert!(!body_text(replay).await.contains("Here is your answer"));

    set_guest_flag(&state, Some(true)).await.unwrap();
    assert!(
        matches!(guest::select_mode(&state.db, &row.channel_bot_id).await.unwrap(),
        guest::GuestMode::AddOn { activation_epoch: current, .. } if current > activation_epoch)
    );
    assert!(
        guest::revalidate(&state.db, active.channel_guest.as_ref().unwrap())
            .await
            .is_err()
    );
    set_guest_flag(&state, None).await.unwrap();
    assert_eq!(
        guest::select_mode(&state.db, &row.channel_bot_id)
            .await
            .unwrap(),
        guest::GuestMode::Legacy
    );
    server.abort();
    state.db.drop().await.unwrap();
}
