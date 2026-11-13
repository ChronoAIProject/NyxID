use super::*;
use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

async fn audit_data(state: &AppState, event: &str, id: &str) -> Value {
    state
        .db
        .collection::<AuditLog>(AUDIT)
        .find_one(doc! {"event_type":event,"event_data.channel_agent_id":id})
        .await
        .unwrap()
        .unwrap()
        .event_data
        .unwrap()
}

async fn retry_http(state: &AppState, row: &NyxbotChannel) -> AppResult<Json<Value>> {
    Box::pin(gateway_retry::retry_channel(
        State(state.clone()),
        crate::test_utils::test_auth_user(OWNER),
        Path(row.id.clone()),
    ))
    .await
}

#[tokio::test]
async fn retry_2183_success_competes_once_and_preserves_binding_and_route() {
    for org_owned in [false, true] {
        let (mut state, _, server) = setup("retry2183_success").await;
        let _server = tokio_util::task::AbortOnDropHandle::new(server);
        let org = if org_owned {
            Some(add_org(&state).await)
        } else {
            None
        };
        let row = linked(&state, org.as_deref(), "discord").await;
        let (_identity, _guard) = native_discord_identity(&state, &row).await;
        let (calls, gateway) = mock_gateway(&mut state, Arc::new(AtomicBool::new(false))).await;
        let _gateway = tokio_util::task::AbortOnDropHandle::new(gateway);
        enable_gateway_for(&state, "discord", OWNER).await;
        due(&state, &row.id).await;
        let (a, b) = tokio::join!(retry_http(&state, &row), retry_http(&state, &row));
        assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
        let result = a.or(b).unwrap().0;
        assert_eq!(result["channel_agent"]["transport"], "gateway");
        assert_eq!(
            result["channel_agent"]["gateway_native_activation"],
            "pending_discovery"
        );
        assert!(
            result["channel_agent"]
                .get("gateway_next_retry_at")
                .is_none()
        );
        let moved = load_channel(&state, OWNER, &row.id).await.unwrap();
        assert_eq!(moved.agent_id, row.agent_id);
        assert_eq!(moved.route_id, row.route_id);
        assert_eq!(moved.owner_sender_ids, row.owner_sender_ids);
        assert_eq!(
            route(&state, &moved).await.user_id,
            org.as_deref().unwrap_or(OWNER)
        );
        assert_eq!(
            calls.lock().await.iter().filter(|c| c.0 == "POST").count(),
            1
        );
        let audit = audit_data(&state, "nyxbot_channel_native_gateway_retried", &row.id).await;
        assert_eq!(audit["transport"], "gateway");
        assert_eq!(audit["gateway_native_activation"], "pending_discovery");
        assert_eq!(audit["moved_to_gateway"], true);
    }
}

#[tokio::test]
async fn retry_2183_rejection_and_connect_fallback_report_real_transport_and_bounded_code() {
    for fresh in [false, true] {
        let (mut state, _, server) = setup("retry2183_rejected").await;
        let _server = tokio_util::task::AbortOnDropHandle::new(server);
        let row = linked(&state, None, "discord").await;
        let (_identity, _guard) = native_discord_identity(&state, &row).await;
        let (_, gateway) = mock_gateway(&mut state, Arc::new(AtomicBool::new(false))).await;
        let _gateway = tokio_util::task::AbortOnDropHandle::new(gateway);
        let cma = MockServer::start().await;
        Mock::given(path("/channel-capabilities")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"relay_authoring":{
            "native_discord_writes_enabled":true,"native_discord_auto_activation_enabled":true,"multi_consumer_writes_enabled":true}}))).expect(1).mount(&cma).await;
        Mock::given(method("POST")).and(path("/channels")).respond_with(ResponseTemplate::new(400).set_body_json(json!({"code":"discord.invalid_intents", "detail":"private upstream text https://secret.example"}))).expect(1).mount(&cma).await;
        state
            .db
            .collection::<DownstreamService>(SERVICES)
            .update_one(
                doc! {"slug":GATEWAY_SLUG},
                doc! {"$set":{"base_url":cma.uri()}},
            )
            .await
            .unwrap();
        enable_gateway_for(&state, "discord", OWNER).await;
        due(&state, &row.id).await;
        let (saved, event) = if fresh {
            state
                .db
                .collection::<NyxbotChannel>(CHANNELS)
                .delete_one(doc! {"_id":&row.id})
                .await
                .unwrap();
            state
                .db
                .collection::<bson::Document>(ROUTES)
                .delete_one(doc! {"_id":&row.route_id})
                .await
                .unwrap();
            let agent = crate::services::assistant_team_service::ensure_nyxbot(&state.db, OWNER)
                .await
                .unwrap();
            (
                Box::pin(connect(&state, OWNER, None, &row.channel_bot_id, &agent))
                    .await
                    .unwrap()
                    .0,
                "nyxbot_channel_connected",
            )
        } else {
            let result = retry_http(&state, &row).await.unwrap().0;
            assert_eq!(result["code"], "gateway_rejected_request");
            assert_eq!(result["moved_to_gateway"], false);
            (
                load_channel(&state, OWNER, &row.id).await.unwrap(),
                "nyxbot_channel_native_gateway_retried",
            )
        };
        assert_eq!(saved.transport, "direct");
        assert_eq!(
            saved.gateway_native_activation.as_deref(),
            Some("gateway_rejected_request")
        );
        let api = json!(ChannelAgentResponse::from(&saved));
        assert_eq!(
            api["gateway_native_activation_detail"],
            "discord.invalid_intents"
        );
        assert!(
            api["gateway_fallback_reason"]
                .as_str()
                .unwrap()
                .starts_with("NyxID relay selected; check channel health")
        );
        assert!(api["gateway_next_retry_at"].is_string());
        assert!(api["gateway_retry_available_at"].is_string());
        assert_eq!(
            api["gateway_next_retry_at"],
            json!(saved.gateway_attempted_at.unwrap() + ChronoDuration::hours(GATEWAY_RETRY_HOURS))
        );
        let mut legacy = saved.clone();
        legacy.gateway_attempted_at = None;
        assert_eq!(
            gateway_retry::next_automatic_retry(&legacy),
            legacy.gateway_fallback_at
        );
        let audit = audit_data(&state, event, &saved.id).await;
        assert_eq!(audit["transport"], "direct");
        assert_eq!(
            audit["gateway_native_activation"],
            "gateway_rejected_request"
        );
        assert_eq!(
            audit["gateway_native_activation_detail"],
            "discord.invalid_intents"
        );
        assert!(!audit.to_string().contains("private upstream"));
        assert!(!audit.to_string().contains("secret.example"));
        if !fresh {
            assert!(
                matches!(retry_http(&state,&saved).await,Err(AppError::Conflict(code)) if code == "native_gateway_retry_rate_limited")
            );
            assert_eq!(
                route(&state, &saved).await.agent_api_key_id,
                row.route_api_key_id
            );
        }
    }
}

#[tokio::test]
async fn retry_2183_preflight_refuses_no_owner_flag_off_answering_and_nonhuman_without_egress() {
    let (mut state, _, server) = setup("retry2183_guards").await;
    let _server = tokio_util::task::AbortOnDropHandle::new(server);
    let row = linked(&state, None, "discord").await;
    let (calls, gateway) = mock_gateway(&mut state, Arc::new(AtomicBool::new(false))).await;
    let _gateway = tokio_util::task::AbortOnDropHandle::new(gateway);
    due(&state, &row.id).await;
    for auth_method in [
        crate::mw::auth::AuthMethod::ApiKey,
        crate::mw::auth::AuthMethod::Delegated,
    ] {
        let mut auth = crate::test_utils::test_auth_user(OWNER);
        auth.auth_method = auth_method;
        assert!(
            gateway_retry::retry_channel(State(state.clone()), auth, Path(row.id.clone()))
                .await
                .is_err()
        );
    }
    assert!(
        gateway_retry::retry_channel(
            State(state.clone()),
            crate::test_utils::test_auth_user(&Uuid::new_v4().to_string()),
            Path(row.id.clone())
        )
        .await
        .is_err()
    );
    assert!(
        matches!(retry_http(&state,&row).await,Err(AppError::Conflict(code)) if code == "flag_changed")
    );
    crate::services::security_readiness_service::ready_test_replica(&state.db).await;
    enable_gateway_for(&state, "discord", OWNER).await;
    state
        .db
        .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .insert_one(doc! {
        "_id":"answering","user_id":OWNER,"channel":{"nyxbot_channel_id":&row.id},
        "active_turn":{"started_at":bson::DateTime::now()}})
        .await
        .unwrap();
    assert!(
        matches!(retry_http(&state,&row).await,Err(AppError::Conflict(code)) if code == "answering")
    );
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(doc! {"_id":&row.id}, doc! {"$set":{"owner_sender_ids":[]}})
        .await
        .unwrap();
    assert!(
        matches!(retry_http(&state,&row).await,Err(AppError::Conflict(code)) if code == "owner_verification_required")
    );
    assert!(calls.lock().await.is_empty());
    assert!(
        load_channel(&state, OWNER, &row.id)
            .await
            .unwrap()
            .gateway_attempted_at
            .is_none()
    );
}

#[tokio::test]
async fn retry_2183_org_admin_revocation_is_live() {
    let (mut state, _, server) = setup("retry2183_org_revoked").await;
    let _server = tokio_util::task::AbortOnDropHandle::new(server);
    let org = add_org(&state).await;
    let row = linked(&state, Some(&org), "discord").await;
    let (calls, gateway) = mock_gateway(&mut state, Arc::new(AtomicBool::new(false))).await;
    let _gateway = tokio_util::task::AbortOnDropHandle::new(gateway);
    crate::services::security_readiness_service::ready_test_replica(&state.db).await;
    enable_gateway_for(&state, "discord", OWNER).await;
    state
        .db
        .collection::<bson::Document>(MEMBERSHIPS)
        .update_one(
            doc! {"org_user_id":&org,"member_user_id":OWNER},
            doc! {"$set":{"role":"member"}},
        )
        .await
        .unwrap();
    assert!(
        matches!(retry_http(&state,&row).await,Err(AppError::Forbidden(code)) if code == "org_access_lost")
    );
    assert!(calls.lock().await.is_empty());
}

#[tokio::test]
async fn retry_2183_tool_is_nyxbot_only_and_uses_channel_management_confirmation() {
    use crate::services::assistant_acknowledgement_service::{
        OwnerActionClass, known_action_class,
    };
    let f = crate::services::assistant_authority_tests::fixture("retry2183_tool").await;
    let args = json!({"channel_agent_id":Uuid::new_v4().to_string()});
    assert_eq!(
        known_action_class("nyxid__retry_channel_gateway", &args),
        Some(OwnerActionClass::Sensitive)
    );
    let (result, error) = crate::handlers::assistant_team::execute_tool(
        &f.state,
        &f.chat,
        "nyxid__retry_channel_gateway",
        &args,
    )
    .await;
    assert!(error);
    assert_eq!(result["error"], "orchestrator_only");
    let mut guest = f.chat.clone();
    guest.guest = true;
    let (result, error) = crate::handlers::assistant_team::execute_tool(
        &f.state,
        &guest,
        "nyxid__retry_channel_gateway",
        &args,
    )
    .await;
    assert!(error);
    assert_eq!(
        result["error"],
        crate::services::assistant_acknowledgement_service::guest_refusal()["error"]
    );
}
