//! Source-only reconciliation coverage; uses the real owner turn and callback auth.
use super::*;
use crate::services::{assistant_service_execution as execution, proxy_service as proxy};

pub(super) async fn ask_every_time_callback_proxy(
    State(state): State<AppState>,
    auth: AuthUser,
    path: Path<(String, String)>,
    request: Request<Body>,
) -> AppResult<Response> {
    let chat = auth.assistant_chat.as_ref().unwrap();
    if !chat.guest {
        assert!(chat.turn_live && !chat.turn_stopped);
        assert_eq!(
            crate::services::assistant_acknowledgement_service::effective_access_mode(
                &state.db, chat
            )
            .await?,
            crate::models::assistant_settings::AccessMode::AskEveryTime,
            "actual authenticated owner callback must use AskEveryTime"
        );
    }
    callback_proxy(State(state), auth, path, request).await
}

pub(super) fn metadata_callback(scenario: &str) -> Value {
    let mut body: Value = serde_json::from_str(include_str!(
        "../services/fixtures/permissions-codex-responses.json"
    ))
    .unwrap();
    let metadata = body["client_metadata"].as_object().unwrap();
    assert_eq!(serde_json::to_vec(metadata).unwrap().len(), 1121);
    for (key, length) in [
        ("root_turn_id", 36),
        ("session_id", 36),
        ("thread_id", 36),
        ("turn_id", 36),
        ("x-codex-installation-id", 36),
        ("x-codex-turn-metadata", 697),
        ("x-codex-window-id", 38),
    ] {
        assert_eq!(metadata[key].as_str().unwrap().len(), length);
    }
    body["model"] = json!("test");
    body["stream"] = json!(false);
    match scenario {
        "metadata_stateful" => body["store"] = json!(true),
        "metadata_hosted" => {
            body["tools"] = json!([{"type":"code_interpreter","container":"auto"}]);
        }
        "metadata_unknown_effect" => body["unknown_effect"] = json!(true),
        "metadata_unknown" => body["client_metadata"]["unknown"] = json!("replacement"),
        "metadata_replacement" => {
            let metadata = body["client_metadata"].as_object_mut().unwrap();
            metadata.remove("x-codex-turn-metadata");
            metadata.insert("unknown".into(), json!("replacement"));
            assert_eq!(metadata.len(), 7);
        }
        "owner_metadata" | "metadata" => {}
        _ => panic!("unrecognized metadata scenario"),
    }
    body
}

#[tokio::test]
async fn codex_bound_guest_metadata_reuses_only_the_real_owner_route() {
    Box::pin(guest_pipeline_with_permissions(
        false,
        &[
            "owner",
            "guest",
            "owner_metadata",
            "metadata",
            "metadata_stateful",
            "metadata_hosted",
            "metadata_unknown",
            "metadata_unknown_effect",
            "metadata_replacement",
            "semantic_header",
            "metadata",
        ],
        true,
    ))
    .await;
}

#[tokio::test]
async fn codex_callback_without_server_model_attempt_cannot_establish_guest_route() {
    Box::pin(guest_pipeline_with_permissions(
        false,
        &["owner_no_attempt", "unbound", "owner_metadata", "metadata"],
        true,
    ))
    .await;
}

#[tokio::test]
async fn codex_bound_guest_admission_revalidate_seam_rejects_stale_input_target_and_route() {
    Box::pin(guest_pipeline_with_permissions(
        false,
        &[
            "owner_metadata",
            "stale_input",
            "metadata",
            "stale_target",
            "metadata",
            "stale_route",
            "unbound",
        ],
        true,
    ))
    .await;
}

// Test-only egress seam: the production AuthUser extractor authenticates the
// callback. Obtain a real Admission, change durable authority, then exercise the
// same revalidate method used immediately before production egress. No provider
// call is substituted/reset, and no authority or route is manufactured here.
pub(super) async fn held_admission(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(mutation): Path<String>,
    request: Request<Body>,
) -> AppResult<Response> {
    let chat = auth.assistant_chat.as_ref().unwrap();
    assert!(chat.guest && chat.turn_live && !chat.turn_stopped);
    let row = state
        .db
        .collection::<bson::Document>("user_services")
        .find_one(doc! {"user_id":&chat.user_id,"slug":"guest-model"})
        .await?
        .unwrap();
    let id = row.get_str("_id").unwrap();
    let mut resolved = proxy::resolve_proxy_target_by_user_service_id(
        &state.db,
        &state.encryption_keys,
        &chat.user_id,
        id,
        Some("guest-model"),
        None,
        proxy::ProxyExecutionContext::new(
            None,
            crate::mw::rate_limit::PlatformUserRateLimitPolicy::disabled(),
        ),
    )
    .await?
    .unwrap();
    let (parts, body) = request.into_parts();
    let body = axum::body::to_bytes(body, 64 * 1024).await.unwrap();
    let selected = engine::get(&state.db, &chat.user_id, &chat.conversation_id).await?;
    assert_eq!(
        selected
            .agent_id
            .as_deref()
            .expect("selected conversation has an agent"),
        chat.agent_id.as_str()
    );
    assert_eq!(selected.credential_api_key_id, chat.api_key_id);
    assert_eq!(
        selected.active_turn.as_ref().unwrap().turn_id.as_str(),
        chat.turn_id.as_deref().unwrap()
    );
    assert_eq!(
        selected.owner_model_route.as_ref().unwrap().profile,
        selected.model
    );
    let agent = state
        .db
        .collection::<bson::Document>("assistant_agents")
        .find_one(doc! {"_id":&chat.agent_id,"user_id":&chat.user_id})
        .await?
        .unwrap();
    assert_eq!(
        agent
            .get_document("owner_model_route")
            .unwrap()
            .get_str("profile")
            .unwrap(),
        selected.model
    );
    let defaults: Value =
        serde_json::from_str(include_str!("../../specs/assistant/model-routes.json")).unwrap();
    let default_slug = defaults[&selected.model]["service_slug"].as_str().unwrap();
    assert_ne!(resolved.target.service.slug, default_slug);
    assert!(resolved.target.service.credential_encrypted.is_empty());
    assert_eq!(
        state
            .db
            .collection::<bson::Document>("downstream_services")
            .count_documents(doc! {"slug":default_slug})
            .await?,
        0,
        "the stale-route seam has no eligible server default to fall back to"
    );
    let decision = execution::check_http(
        &state,
        &auth,
        Some(id),
        Some(&chat.user_id),
        &resolved.target,
        "POST",
        "v1/responses",
        None,
        &body,
        None,
        None,
        &parts.headers,
    )
    .await?;
    assert!(decision.refusal.is_none());
    let admission = decision.admission.expect("bound guest gets real admission");
    admission
        .revalidate(&state.db, id, Some(&resolved.target))
        .await?;
    match mutation.as_str() {
        "input" => {
            let changed = state
                .db
                .collection::<bson::Document>("assistant_service_execution")
                .update_one(
                    doc! {"user_id":&chat.user_id,"conversation_id":&chat.conversation_id,
                    "turn_id":&chat.turn_id,"kind":"input_admission"},
                    doc! {"$set":{"input_admission":Uuid::new_v4().to_string()}},
                )
                .await?;
            assert_eq!(changed.matched_count, 1);
        }
        "target" => {
            resolved.target.base_url.push_str("/changed-target");
        }
        "route" => {
            let changed = state
                .db
                .collection::<bson::Document>("assistant_conversations")
                .update_one(
                    doc! {"_id":&chat.conversation_id,"user_id":&chat.user_id,
                    "agent_id":&chat.agent_id,"credential_api_key_id":&chat.api_key_id,
                    "active_turn.turn_id":&chat.turn_id,"guest_turn":true},
                    doc! {"$unset":{"owner_model_route":""}},
                )
                .await?;
            assert_eq!(changed.matched_count, 1);
            assert_eq!(changed.modified_count, 1);
            let changed = state
                .db
                .collection::<bson::Document>("assistant_agents")
                .update_one(
                    doc! {"_id":&chat.agent_id,"user_id":&chat.user_id},
                    doc! {"$unset":{"owner_model_route":""}},
                )
                .await?;
            assert_eq!(changed.modified_count, 1);
            assert!(
                engine::get(&state.db, &chat.user_id, &chat.conversation_id)
                    .await?
                    .owner_model_route
                    .is_none()
            );
        }
        _ => panic!("unrecognized authority mutation"),
    }
    let result = admission
        .revalidate(&state.db, id, Some(&resolved.target))
        .await;
    assert!(matches!(result, Err(AppError::GuestModelRouteRequired)));
    result?;
    panic!("stale admission unexpectedly revalidated");
}
