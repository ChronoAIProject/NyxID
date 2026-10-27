use super::*;
use crate::models::api_key::COLLECTION_NAME as KEYS;
use crate::{
    models::nyxbot_gateway_release::{
        COLLECTION_NAME as RELEASES, NyxbotGatewayRelease as Release, ReleaseState,
    },
    services::{
        nyxbot_channel_continuity_service as continuity, nyxbot_gateway_release_service as release,
    },
};
use std::sync::atomic::AtomicBool;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

async fn fixture(
    name: &str,
) -> (
    AppState,
    NyxbotChannel,
    String,
    MockServer,
    tokio::task::JoinHandle<()>,
) {
    let (mut state, _, upstream) = setup(name).await;
    let server = MockServer::start().await;
    state.config.jwt_issuer = "http://localhost:3001".into();
    let mut service = crate::models::downstream_service::test_helpers::dummy_service();
    service.id = Uuid::new_v4().to_string();
    service.slug = "cmaeg".into();
    service.base_url = server.uri();
    service.requires_user_credential = false;
    state
        .db
        .collection::<DownstreamService>(SERVICES)
        .insert_one(service)
        .await
        .unwrap();
    let (mut row, key) = channel(&state, "gateway").await;
    row.status = "pending".into();
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&row.id}, &row)
        .await
        .unwrap();
    (state, row, key, server, upstream)
}
async fn reserve(state: &AppState, row: &NyxbotChannel) -> Release {
    release::reserve(
        state,
        row,
        doc! {"_id":&row.id,"status":"pending","route_api_key_id":&row.route_api_key_id},
    )
    .await
    .unwrap()
}
fn remote(row: &Release, version: i64) -> Value {
    json!({"record_id":row.record_id,"channel_id":"cmaeg1.ch.remote","version":version,
        "provider":{"slug":"nyxbot","binding_id":row.binding_id},
        "definition":{"owner":{"kind":"human","subject":row.owner_user_id},
            "sources":[{"type":"nyxid_relay","issuer":row.creator_issuer,"key_id":row.route_api_key_id,"platform":row.platform}]}})
}
async fn current(state: &AppState, id: &str) -> Release {
    release::get(&state.db, id).await.unwrap()
}
async fn assert_retired_key(state: &AppState, id: &str) {
    let key = state
        .db
        .collection::<crate::models::api_key::ApiKey>(KEYS)
        .find_one(doc! {"_id":id,"user_id":OWNER})
        .await
        .unwrap()
        .unwrap();
    assert!(!key.is_active);
    assert!(!key.key_hash.is_empty());
}
async fn immediate(state: &AppState, id: &str) {
    state
        .db
        .collection::<Release>(RELEASES)
        .update_one(
            doc! {"_id":id},
            doc! {"$set":{"next_attempt_at":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
}
async fn execute(state: &AppState, id: &str) -> bool {
    immediate(state, id).await;
    let claimed = release::claim(&state.db, Some(id)).await.unwrap().unwrap();
    release::execute(state, claimed).await.unwrap()
}
async fn mount_remote(server: &MockServer, row: &Release, version: i64) {
    Mock::given(method("GET"))
        .and(path(format!(
            "/channel-records/{}",
            row.record_id.as_deref().unwrap()
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(remote(row, version)))
        .mount(server)
        .await;
}
async fn expire(state: &AppState, id: &str) {
    state
        .db
        .collection::<Release>(RELEASES)
        .update_one(
            doc! {"_id":id},
            doc! {"$set":{"provisional_expires_at":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn release_crash_boundaries_reservation_dispatch_post_attachment_and_activation() {
    for phase in 0..4 {
        let (state, row, _, server, upstream) = fixture("release_crashes").await;
        let reserved = reserve(&state, &row).await;
        assert_eq!(reserved.id, reserved.record_id.as_deref().unwrap());
        assert_eq!(
            load_channel(&state, OWNER, &row.id)
                .await
                .unwrap()
                .gateway_setup_release_id
                .as_deref(),
            Some(reserved.id.as_str())
        );
        if phase > 0 {
            release::dispatch(&state.db, &reserved).await.unwrap();
            assert!(release::dispatch(&state.db, &reserved).await.is_err());
        }
        if phase > 1 {
            release::record_ack(&state.db, &reserved, "cmaeg1.ch.remote", phase)
                .await
                .unwrap();
        }
        expire(&state, &reserved.id).await;
        assert!(
            release::record_ack(&state.db, &reserved, "cmaeg1.ch.remote", 5)
                .await
                .is_err()
        );
        release::retire_attempt(&state, &reserved.id, ReleaseReason::StaleSetup)
            .await
            .unwrap();
        let retired = current(&state, &reserved.id).await;
        assert_eq!(
            retired.state,
            if phase == 0 {
                ReleaseState::Settled
            } else {
                ReleaseState::Due
            }
        );
        assert_eq!(
            retired.expected_version,
            if phase > 1 { Some(phase) } else { None }
        );
        let failed = load_channel(&state, OWNER, &row.id).await.unwrap();
        assert_eq!(failed.status, "failed");
        assert_eq!(failed.last_error.as_deref(), Some("setup_interrupted"));
        assert!(failed.agent_key_ciphertext.is_none());
        assert!(failed.gateway_setup_release_id.is_none());
        assert_retired_key(&state, &row.route_api_key_id).await;
        assert!(server.received_requests().await.unwrap().is_empty());
        upstream.abort();
        state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn release_uncertain_create_404_escalates_then_late_record_is_attributable() {
    let (state, row, _, server, upstream) = fixture("release_uncertain").await;
    let reserved = reserve(&state, &row).await;
    release::dispatch(&state.db, &reserved).await.unwrap();
    release::retire_attempt(&state, &reserved.id, ReleaseReason::SetupFailed)
        .await
        .unwrap();
    assert!(!execute(&state, &reserved.id).await);
    assert_eq!(current(&state, &reserved.id).await.state, ReleaseState::Due);
    state.db.collection::<Release>(RELEASES).update_one(doc! {"_id":&reserved.id},doc! {"$set":{"dispatch_started_at":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::hours(25))}}).await.unwrap();
    assert!(!execute(&state, &reserved.id).await);
    let review = current(&state, &reserved.id).await;
    assert_eq!(review.state, ReleaseState::ReviewRequired);
    assert_eq!(
        review.last_error.as_deref(),
        Some("create_outcome_unconfirmed")
    );
    assert!(
        release::claim(&state.db, Some(&reserved.id))
            .await
            .unwrap()
            .is_none()
    );
    mount_remote(&server, &reserved, 1).await;
    assert!(matches!(
        release::observe(&state, &review).await.unwrap(),
        release::Observation::Matching { version: 1, .. }
    ));
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method != "DELETE" && r.method != "POST")
    );
    upstream.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn release_changed_version_and_lost_put_never_delete_observed_version() {
    let (state, row, _, server, upstream) = fixture("release_changed").await;
    let reserved = reserve(&state, &row).await;
    release::dispatch(&state.db, &reserved).await.unwrap();
    release::record_ack(&state.db, &reserved, "cmaeg1.ch.remote", 1)
        .await
        .unwrap();
    release::retire_attempt(&state, &reserved.id, ReleaseReason::SetupFailed)
        .await
        .unwrap();
    mount_remote(&server, &reserved, 2).await;
    assert!(!execute(&state, &reserved.id).await);
    let review = current(&state, &reserved.id).await;
    assert_eq!(review.state, ReleaseState::ReviewRequired);
    assert_eq!(review.expected_version, Some(1));
    assert_eq!(review.observed_version, Some(2));
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method != "DELETE")
    );
    upstream.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn release_lease_takeover_fences_stale_completion_and_repeated_delete_404() {
    let (state, row, _, server, upstream) = fixture("release_lease").await;
    let reserved = reserve(&state, &row).await;
    release::dispatch(&state.db, &reserved).await.unwrap();
    release::record_ack(&state.db, &reserved, "cmaeg1.ch.remote", 1)
        .await
        .unwrap();
    release::retire_attempt(&state, &reserved.id, ReleaseReason::SetupFailed)
        .await
        .unwrap();
    let old = release::claim(&state.db, Some(&reserved.id))
        .await
        .unwrap()
        .unwrap();
    state
        .db
        .collection::<Release>(RELEASES)
        .update_one(
            doc! {"_id":&reserved.id},
            doc! {"$set":{"lease_until":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    let next = release::claim(&state.db, Some(&reserved.id))
        .await
        .unwrap()
        .unwrap();
    assert_ne!(old.claim_id, next.claim_id);
    assert!(next.revision > old.revision);
    // Default 404 is conclusive for the captured channel, including a lost DELETE reply.
    assert!(!release::execute(&state, old).await.unwrap());
    assert!(release::execute(&state, next).await.unwrap());
    let settled = current(&state, &reserved.id).await;
    assert_eq!(settled.state, ReleaseState::Settled);
    assert!(
        release::claim(&state.db, Some(&reserved.id))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method != "DELETE")
    );
    upstream.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn release_recovery_identity_destination_owner_and_live_reference_fail_closed() {
    for mode in [
        "owner",
        "owner_kind",
        "binding",
        "source",
        "destination",
        "missing_destination",
        "inactive_destination",
        "misconfigured_destination",
        "issuer_drift",
        "live",
        "unavailable_owner",
        "malformed_json",
    ] {
        let (mut state, row, _, server, upstream) = fixture("release_identity").await;
        let reserved = reserve(&state, &row).await;
        release::dispatch(&state.db, &reserved).await.unwrap();
        release::retire_attempt(&state, &reserved.id, ReleaseReason::SetupFailed)
            .await
            .unwrap();
        let mut body = remote(&reserved, 1);
        match mode {
            "owner" => body["definition"]["owner"]["subject"] = json!(Uuid::new_v4().to_string()),
            "owner_kind" => body["definition"]["owner"]["kind"] = json!("service"),
            "binding" => body["provider"]["binding_id"] = json!("bnd_other"),
            "source" => body["definition"]["sources"][0]["key_id"] = json!("other"),
            "destination" => {
                state
                    .db
                    .collection::<DownstreamService>(SERVICES)
                    .update_one(
                        doc! {"slug":"cmaeg"},
                        doc! {"$set":{"base_url":"https://substituted.example"}},
                    )
                    .await
                    .unwrap();
            }
            "missing_destination" => {
                state
                    .db
                    .collection::<DownstreamService>(SERVICES)
                    .delete_one(doc! {"slug":"cmaeg"})
                    .await
                    .unwrap();
            }
            "inactive_destination" => {
                state
                    .db
                    .collection::<DownstreamService>(SERVICES)
                    .update_one(doc! {"slug":"cmaeg"}, doc! {"$set":{"is_active":false}})
                    .await
                    .unwrap();
            }
            "misconfigured_destination" => {
                state
                    .db
                    .collection::<DownstreamService>(SERVICES)
                    .update_one(
                        doc! {"slug":"cmaeg"},
                        doc! {"$set":{"requires_user_credential":true}},
                    )
                    .await
                    .unwrap();
            }
            "issuer_drift" => state.config.jwt_issuer = "https://changed-issuer.example".into(),
            "live" => {
                state.db.collection::<NyxbotChannel>(CHANNELS).update_one(doc! {"_id":&row.id},doc! {"$set":{"status":"active","agent_api_key_id":&reserved.agent_api_key_id}}).await.unwrap();
            }
            "unavailable_owner" => {
                state
                    .db
                    .collection::<bson::Document>(USERS)
                    .update_one(doc! {"_id":OWNER}, doc! {"$set":{"is_active":false}})
                    .await
                    .unwrap();
            }
            "malformed_json" => {}
            _ => unreachable!(),
        }
        let response = if mode == "malformed_json" {
            ResponseTemplate::new(200).set_body_string("{")
        } else {
            ResponseTemplate::new(200).set_body_json(body)
        };
        Mock::given(method("GET"))
            .respond_with(response)
            .mount(&server)
            .await;
        assert!(!execute(&state, &reserved.id).await);
        assert_eq!(
            current(&state, &reserved.id).await.state,
            ReleaseState::ReviewRequired
        );
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method != "DELETE")
        );
        upstream.abort();
        state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn release_retired_primary_only_acknowledges_its_exact_binding() {
    let (state, row, raw, _, upstream) = fixture("release_unbind").await;
    let reserved = reserve(&state, &row).await;
    release::dispatch(&state.db, &reserved).await.unwrap();
    release::retire_attempt(&state, &reserved.id, ReleaseReason::SetupFailed)
        .await
        .unwrap();
    use release::RetiredAcknowledgement as Ack;
    assert_eq!(
        release::acknowledge_retired(&state, &raw, &reserved.binding_id).await,
        Ack::Acknowledged
    );
    assert_eq!(
        release::acknowledge_retired(&state, &raw, "other").await,
        Ack::NotApplicable
    );
    assert_eq!(
        release::acknowledge_retired(&state, "unknown", &reserved.binding_id).await,
        Ack::NotApplicable
    );
    assert_eq!(
        delete_binding(
            State(state.clone()),
            Path(reserved.binding_id.clone()),
            bearer(&raw)
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert!(provider_channel(&state, &bearer(&raw)).await.is_err());
    assert_eq!(
        put_binding(
            State(state.clone()),
            Path(reserved.binding_id.clone()),
            bearer(&raw),
            Json(binding_body(&raw, OWNER))
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        put_conversation(
            State(state.clone()),
            Path((reserved.binding_id.clone(), PARTITION.into())),
            bearer(&raw)
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    state
        .db
        .collection::<bson::Document>(KEYS)
        .update_one(
            doc! {"_id":&reserved.agent_api_key_id},
            doc! {"$set":{"user_id":"wrong-owner"}},
        )
        .await
        .unwrap();
    assert_eq!(
        release::acknowledge_retired(&state, &raw, &reserved.binding_id).await,
        Ack::NotApplicable
    );
    state
        .db
        .collection::<bson::Document>(KEYS)
        .update_one(
            doc! {"_id":&reserved.agent_api_key_id},
            doc! {"$set":{"user_id":OWNER}},
        )
        .await
        .unwrap();
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"status":"active","pending_agent_api_key_id":&reserved.agent_api_key_id}},
        )
        .await
        .unwrap();
    assert_eq!(
        release::acknowledge_retired(&state, &raw, &reserved.binding_id).await,
        Ack::ConflictingLiveReference
    );
    assert_eq!(
        delete_binding(
            State(state.clone()),
            Path(reserved.binding_id.clone()),
            bearer(&raw)
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    upstream.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn release_lost_post_recovers_record_and_cma_delete_requires_retired_unbind() {
    let (mut state, _, upstream) = setup("release_cma_order").await;
    let (calls, gateway) = mock_gateway(&mut state, Arc::new(AtomicBool::new(false))).await;
    let (mut row, raw) = channel(&state, "gateway").await;
    row.status = "pending".into();
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&row.id}, &row)
        .await
        .unwrap();
    let bot = channel_bot_service::get_bot(&state.db, &row.channel_bot_id)
        .await
        .unwrap();
    let reserved = reserve(&state, &row).await;
    release::dispatch(&state.db, &reserved).await.unwrap();
    let mut body = gateway_policy(&state, &row, &bot, &[], GATEWAY_GROUPS_DEFAULT);
    body["record_id"] = json!(reserved.record_id);
    body["credentials"] = json!({"agent_key":raw,"channel_key":"test-only-route-key"});
    let creator = creator_bearer(&state, OWNER).unwrap();
    let response = gateway_call(
        &state,
        reqwest::Method::POST,
        "/channels",
        &creator,
        Some(&body),
        Some(&reserved.id),
    )
    .await
    .unwrap();
    assert_eq!(response.status, 201);
    drop(response); // crash/lost POST response before any acknowledgement
    release::retire_attempt(&state, &reserved.id, ReleaseReason::SetupFailed)
        .await
        .unwrap();
    assert!(execute(&state, &reserved.id).await);
    let settled = current(&state, &reserved.id).await;
    assert_eq!(settled.expected_version, Some(1));
    assert_eq!(settled.state, ReleaseState::Settled);
    let methods = calls
        .lock()
        .await
        .iter()
        .map(|c| c.0.clone())
        .collect::<Vec<_>>();
    assert_eq!(methods, ["POST", "GET", "DELETE"]);
    assert!(
        provider_channel(
            &state,
            &bearer(body["credentials"]["agent_key"].as_str().unwrap())
        )
        .await
        .is_err()
    );
    assert_eq!(
        release::observe(&state, &settled).await.unwrap(),
        release::Observation::Missing
    );
    gateway.abort();
    upstream.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn release_retirement_and_route_key_revocation_roll_back_together() {
    let (state, mut row, _, _, upstream) = fixture("release_atomic_retire").await;
    row.status = "active".into();
    row.gateway_record_id = Some(Uuid::new_v4().to_string());
    row.gateway_channel_id = Some("cmaeg1.ch.remote".into());
    row.gateway_version = Some(1);
    row.binding_id = Some("exact-binding".into());
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&row.id}, &row)
        .await
        .unwrap();
    state
        .db
        .run_command(doc! {"collMod":CHANNELS,"validator":{"status":{"$ne":"disconnected"}}})
        .await
        .unwrap();
    assert!(
        release::retire_connection(
            &state,
            &row,
            ReleaseReason::Disconnect,
            "disconnected",
            None
        )
        .await
        .is_err()
    );
    assert_eq!(
        state
            .db
            .collection::<Release>(RELEASES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        load_channel(&state, OWNER, &row.id).await.unwrap().status,
        "active"
    );
    assert!(
        key_service::get_api_key(&state.db, OWNER, &row.route_api_key_id)
            .await
            .unwrap()
            .is_active
    );
    assert!(
        state
            .db
            .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
            .find_one(doc! {"_id":&row.route_id,"is_active":true})
            .await
            .unwrap()
            .is_some()
    );
    state
        .db
        .run_command(doc! {"collMod":CHANNELS,"validator":{}})
        .await
        .unwrap();
    let response = disconnect(&state, OWNER, &row.id).await.unwrap();
    assert_eq!(response["gateway_released"], true);
    let retired = state
        .db
        .collection::<Release>(RELEASES)
        .find_one(doc! {"channel_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retired.owner_user_id, OWNER);
    assert_eq!(retired.expected_version, Some(1));
    assert_eq!(retired.state, ReleaseState::Settled);
    upstream.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn release_disconnect_org_loss_fallback_and_setup_failure_capture_pending_attempts() {
    for mode in ["disconnect", "org", "fallback", "setup_failure"] {
        let (state, row, _, _, upstream) = fixture("release_paths").await;
        let reserved = reserve(&state, &row).await;
        release::dispatch(&state.db, &reserved).await.unwrap();
        match mode {
            "disconnect" => {
                disconnect(&state, OWNER, &row.id).await.unwrap();
            }
            "org" => {
                let mut org = row.clone();
                org.bot_owner_id = Some(OWNER.into());
                release_org_channel(&state, &org).await.unwrap();
            }
            "fallback" => {
                let fresh = key(&state, "fresh direct").await;
                release::fallback(&state, &row, &fresh.id).await.unwrap();
                let direct = load_channel(&state, OWNER, &row.id).await.unwrap();
                assert_eq!(direct.transport, "direct");
                assert_eq!(direct.route_api_key_id, fresh.id);
                assert!(
                    key_service::get_api_key(&state.db, OWNER, &fresh.id)
                        .await
                        .unwrap()
                        .is_active
                );
            }
            _ => {
                release::retire_attempt(&state, &reserved.id, ReleaseReason::SetupFailed)
                    .await
                    .unwrap();
            }
        }
        let retired = current(&state, &reserved.id).await;
        assert_eq!(retired.state, ReleaseState::Due);
        assert_eq!(retired.owner_user_id, OWNER);
        assert_retired_key(&state, &reserved.agent_api_key_id).await;
        assert_retired_key(&state, &reserved.route_api_key_id).await;
        assert!(
            load_channel(&state, OWNER, &row.id)
                .await
                .unwrap()
                .gateway_setup_release_id
                .is_none()
        );
        upstream.abort();
        state.db.drop().await.unwrap();
    }
    // A legacy gateway row without retained identity cannot prove release.
    let (state, row, _, server, upstream) = fixture("release_unknown_legacy").await;
    let response = disconnect(&state, OWNER, &row.id).await.unwrap();
    assert_eq!(response["gateway_released"], false);
    assert!(server.received_requests().await.unwrap().is_empty());
    upstream.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn release_activation_and_stale_reclaimer_have_one_winner() {
    for expired in [false, true] {
        let (state, row, _, _, upstream) = fixture("release_adoption").await;
        let reserved = reserve(&state, &row).await;
        release::dispatch(&state.db, &reserved).await.unwrap();
        release::record_ack(&state.db, &reserved, "cmaeg1.ch.remote", 2)
            .await
            .unwrap();
        state.db.collection::<NyxbotChannel>(CHANNELS).update_one(doc! {"_id":&row.id},doc! {"$set":{"gateway_channel_id":"cmaeg1.ch.remote","gateway_record_id":&reserved.record_id,"gateway_version":2_i64}}).await.unwrap();
        if expired {
            expire(&state, &reserved.id).await;
        }
        let activated = continuity::activate(
            &state.db,
            OWNER,
            &row.id,
            row.route_id.as_deref().unwrap(),
            None,
            true,
            "fresh-hash",
        )
        .await;
        if expired {
            assert!(activated.is_err());
            release::retire_attempt(&state, &reserved.id, ReleaseReason::StaleSetup)
                .await
                .unwrap();
            assert_eq!(current(&state, &reserved.id).await.state, ReleaseState::Due);
        } else {
            activated.unwrap();
            release::retire_attempt(&state, &reserved.id, ReleaseReason::StaleSetup)
                .await
                .unwrap();
            assert!(release::get(&state.db, &reserved.id).await.is_err());
            let active = load_channel(&state, OWNER, &row.id).await.unwrap();
            assert_eq!(active.status, "active");
            assert!(active.gateway_setup_release_id.is_none());
            assert!(active.gateway_groups_retry_at.is_some());
        }
        upstream.abort();
        state.db.drop().await.unwrap();
    }
}

async fn direct_successor(state: &AppState, previous: &NyxbotChannel) -> NyxbotChannel {
    let (mut next, _) = channel(state, "direct").await;
    next.channel_bot_id = previous.channel_bot_id.clone();
    next.status = "pending".into();
    next.platform = previous.platform.clone();
    next.agent_id = previous.agent_id.clone();
    next.agent_api_key_id = None;
    next.created_at = Utc::now();
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&next.id}, &next)
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&next.route_id},
            doc! {"$set":{"channel_bot_id":&previous.channel_bot_id}},
        )
        .await
        .unwrap();
    next
}
async fn thread_doc(
    state: &AppState,
    channel: &NyxbotChannel,
    partition: &str,
    child: bool,
) -> String {
    let id = Uuid::new_v4().to_string();
    let mut row = doc! {"_id":&id,"channel_id":&channel.id,"user_id":OWNER,"partition":partition,"conversation_id":"old-conversation",
    "agent_id":bson::Bson::Null,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()};
    if child {
        row.insert("record_scope", "platform_thread");
        row.insert("follow_state", "active");
    }
    state
        .db
        .collection::<bson::Document>(THREADS)
        .insert_one(row)
        .await
        .unwrap();
    id
}

#[tokio::test]
async fn continuity_restores_stable_chats_policy_verified_owners_and_both_reply_metadata() {
    for changed in [false, true] {
        let (state, mut previous, _, _, upstream) = fixture("continuity_restore").await;
        previous.transport = "direct".into();
        previous.platform = "lark".into();
        previous.status = "disconnected".into();
        previous.owner_sender_ids = vec!["verified-owner".into()];
        previous.private_chats = Some("everyone".into());
        previous.agent_id = Some("agent-old".into());
        previous.link_code_hash = Some("never-copy".into());
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .replace_one(doc! {"_id":&previous.id}, &previous)
            .await
            .unwrap();
        let stable = thread_doc(&state, &previous, "chat_stable", false).await;
        let opaque = thread_doc(&state, &previous, PARTITION, false).await;
        let child = thread_doc(&state, &previous, "chat_child", true).await;
        let mut session = state.db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        crate::services::channel_thread_follow_service::suspend_in_session(
            &state.db,
            OWNER,
            &previous.id,
            "disconnected",
            &mut session,
        )
        .await
        .unwrap();
        session.commit_transaction().await.unwrap();
        let mut next = direct_successor(&state, &previous).await;
        next.owner_sender_ids = vec!["newly-verified-owner".into(), "verified-owner".into()];
        if changed {
            next.agent_id = Some("agent-new".into());
        }
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .replace_one(doc! {"_id":&next.id}, &next)
            .await
            .unwrap();
        let conversations = state.db.collection::<bson::Document>(CONVERSATIONS);
        conversations.insert_one(doc! {"_id":"metadata-only","user_id":OWNER,"channel":{"nyxbot_channel_id":&previous.id,"partition":"chat_stable"},
            "reply_channel":{"nyxbot_channel_id":&previous.id,"partition":PARTITION}}).await.unwrap();
        conversations.insert_one(doc! {"_id":"reply-only","user_id":OWNER,"reply_channel":{"nyxbot_channel_id":&previous.id,"partition":"chat_stable"}}).await.unwrap();
        continuity::activate(
            &state.db,
            OWNER,
            &next.id,
            next.route_id.as_deref().unwrap(),
            None,
            false,
            "fresh-link-hash",
        )
        .await
        .unwrap();
        let active = load_channel(&state, OWNER, &next.id).await.unwrap();
        assert_eq!(
            active.owner_sender_ids,
            ["newly-verified-owner", "verified-owner"]
        );
        assert_eq!(active.private_chats, previous.private_chats);
        assert_eq!(active.link_code_hash.as_deref(), Some("fresh-link-hash"));
        assert_eq!(
            active.link_code_expires_at.unwrap() - ChronoDuration::hours(LINK_CODE_TTL_HOURS),
            active.updated_at
        );
        assert_eq!(
            active.continuity_from_channel_id.as_deref(),
            Some(previous.id.as_str())
        );
        assert!(active.continuity_restored_at.is_some());
        assert_eq!(
            load_channel(&state, OWNER, &previous.id)
                .await
                .unwrap()
                .continuity_to_channel_id
                .as_deref(),
            Some(next.id.as_str())
        );
        let threads = state.db.collection::<bson::Document>(THREADS);
        let moved = threads
            .find_one(doc! {"_id":&stable})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(moved.get_str("channel_id").unwrap(), next.id);
        assert_eq!(
            moved.get_str("conversation_id").ok(),
            if changed {
                None
            } else {
                Some("old-conversation")
            }
        );
        assert_eq!(
            threads
                .find_one(doc! {"_id":opaque})
                .await
                .unwrap()
                .unwrap()
                .get_str("channel_id")
                .unwrap(),
            previous.id
        );
        let followed = threads.find_one(doc! {"_id":child}).await.unwrap().unwrap();
        assert_eq!(
            followed.get_str("follow_state").unwrap(),
            if changed { "stopped" } else { "unavailable" }
        );
        let meta = conversations
            .find_one(doc! {"_id":"metadata-only"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            meta.get_document("channel")
                .unwrap()
                .get_str("nyxbot_channel_id")
                .unwrap(),
            next.id
        );
        assert_eq!(
            meta.get_document("reply_channel")
                .unwrap()
                .get_str("nyxbot_channel_id")
                .unwrap(),
            previous.id
        );
        assert_eq!(
            conversations
                .find_one(doc! {"_id":"reply-only"})
                .await
                .unwrap()
                .unwrap()
                .get_document("reply_channel")
                .unwrap()
                .get_str("nyxbot_channel_id")
                .unwrap(),
            next.id
        );
        let audit = state.db.collection::<crate::models::audit_log::AuditLog>(crate::models::audit_log::COLLECTION_NAME)
            .find_one(doc! {"event_type":"nyxbot_channel_continuity_restored","event_data.channel_id":&next.id})
            .await.unwrap().unwrap().event_data.unwrap();
        assert_eq!(audit["moved"], 2);
        assert_eq!(audit["kept_conflicting"], 0);
        upstream.abort();
        state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn continuity_failed_restoration_and_disconnect_cannot_consume_predecessor_or_revive_setup() {
    let (state, mut previous, _, _, upstream) = fixture("continuity_rollback").await;
    previous.status = "disconnected".into();
    previous.transport = "direct".into();
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&previous.id}, &previous)
        .await
        .unwrap();
    let next = direct_successor(&state, &previous).await;
    state
        .db
        .run_command(doc! {"collMod":CHANNELS,"validator":{"status":{"$ne":"active"}}})
        .await
        .unwrap();
    assert!(
        continuity::activate(
            &state.db,
            OWNER,
            &next.id,
            next.route_id.as_deref().unwrap(),
            None,
            true,
            "fresh"
        )
        .await
        .is_err()
    );
    assert!(
        load_channel(&state, OWNER, &previous.id)
            .await
            .unwrap()
            .continuity_to_channel_id
            .is_none()
    );
    assert_eq!(
        load_channel(&state, OWNER, &next.id).await.unwrap().status,
        "pending"
    );
    state
        .db
        .run_command(doc! {"collMod":CHANNELS,"validator":{}})
        .await
        .unwrap();
    disconnect(&state, OWNER, &next.id).await.unwrap();
    assert!(
        continuity::activate(
            &state.db,
            OWNER,
            &next.id,
            next.route_id.as_deref().unwrap(),
            None,
            true,
            "fresh"
        )
        .await
        .is_err()
    );
    assert_eq!(
        load_channel(&state, OWNER, &next.id).await.unwrap().status,
        "disconnected"
    );
    upstream.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn release_reconcile_resumes_pages_serializes_replicas_and_discovers_later_retirements() {
    let (state, mut template, _, _, upstream) = fixture("release_reconcile").await;
    template.status = "disconnected".into();
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&template.id}, &template)
        .await
        .unwrap();
    let mut ids = Vec::new();
    for index in 0..103 {
        let mut row = template.clone();
        row.id = Uuid::new_v4().to_string();
        row.gateway_record_id = Some(Uuid::new_v4().to_string());
        row.gateway_channel_id = Some(format!("remote-{index}"));
        row.gateway_version = Some(1);
        row.binding_id = Some(format!("binding-{index}"));
        row.agent_api_key_id = Some(format!("agent-{index}"));
        row.route_api_key_id = format!("route-{index}");
        if index == 101 {
            row.status = "active".into();
        }
        if index == 102 {
            state.db.collection::<bson::Document>(crate::models::platform_bot_config::CLEANUP_COLLECTION).insert_one(doc! {"_id":"conversion","kind":"gateway","resource_id":&row.id,"owner_user_id":OWNER}).await.unwrap();
        }
        ids.push(row.id.clone());
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .insert_one(row)
            .await
            .unwrap();
    }
    let (a, b) = tokio::join!(
        release::reconcile_step(&state),
        release::reconcile_step(&state)
    );
    a.unwrap();
    b.unwrap();
    release::reconcile_step(&state).await.unwrap();
    assert_eq!(
        state
            .db
            .collection::<Release>(RELEASES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        101
    );
    assert!(
        state
            .db
            .collection::<Release>(RELEASES)
            .find_one(doc! {"channel_id":&ids[101]})
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        state
            .db
            .collection::<Release>(RELEASES)
            .find_one(doc! {"channel_id":&ids[102]})
            .await
            .unwrap()
            .is_none()
    );
    // An old writer retains identity but knows nothing of release obligations.
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id":&ids[101]},
            doc! {"$set":{"status":"disconnected"}},
        )
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>("schema_migrations")
        .update_one(
            doc! {"_id":crate::models::nyxbot_gateway_release::RECONCILE_ID},
            doc! {"$set":{"pass_completed_at":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    release::reconcile_step(&state).await.unwrap();
    release::reconcile_step(&state).await.unwrap();
    assert_eq!(
        state
            .db
            .collection::<Release>(RELEASES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        102
    );
    let before = state
        .db
        .collection::<Release>(RELEASES)
        .find_one(doc! {"channel_id":&ids[0]})
        .await
        .unwrap()
        .unwrap();
    state
        .db
        .collection::<Release>(RELEASES)
        .update_one(
            doc! {"_id":&before.id},
            doc! {"$set":{"state":"settled","resolution":"preserved_changed"}},
        )
        .await
        .unwrap();
    // Duplicate retained history must reuse the same settled obligation,
    // including a legacy row that retained only the remote channel identity.
    let original = load_channel(&state, OWNER, &ids[0]).await.unwrap();
    for retained_record in [true, false] {
        let mut duplicate = original.clone();
        duplicate.id = Uuid::new_v4().to_string();
        if !retained_record {
            duplicate.gateway_record_id = None;
        }
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .insert_one(duplicate)
            .await
            .unwrap();
    }
    state
        .db
        .collection::<bson::Document>("schema_migrations")
        .update_one(
            doc! {"_id":crate::models::nyxbot_gateway_release::RECONCILE_ID},
            doc! {"$set":{"pass_completed_at":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    release::reconcile_step(&state).await.unwrap();
    release::reconcile_step(&state).await.unwrap();
    assert_eq!(
        current(&state, &before.id).await.state,
        ReleaseState::Settled
    );
    assert_eq!(
        state
            .db
            .collection::<Release>(RELEASES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        102
    );
    upstream.abort();
    state.db.drop().await.unwrap();
}

async fn admin(state: &AppState) {
    crate::services::role_service::seed_system_roles(&state.db)
        .await
        .unwrap();
    let roles = crate::services::role_service::get_platform_role_ids(&state.db)
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>(USERS)
        .update_one(doc! {"_id":OWNER}, doc! {"$set":{"role_ids":[roles.admin]}})
        .await
        .unwrap();
}

#[tokio::test]
async fn release_admin_fresh_observation_revision_preserve_and_confirm_are_fenced() {
    let (state, row, _, server, upstream) = fixture("release_admin").await;
    admin(&state).await;
    let reserved = reserve(&state, &row).await;
    release::dispatch(&state.db, &reserved).await.unwrap();
    release::record_ack(&state.db, &reserved, "cmaeg1.ch.remote", 1)
        .await
        .unwrap();
    release::retire_attempt(&state, &reserved.id, ReleaseReason::SetupFailed)
        .await
        .unwrap();
    mount_remote(&server, &reserved, 2).await;
    assert!(!execute(&state, &reserved.id).await);
    let review = current(&state, &reserved.id).await;
    let observed = release::observe(&state, &review).await.unwrap();
    assert!(
        release::resolve(
            &state,
            OWNER,
            &review.id,
            review.revision - 1,
            observed.clone(),
            release::Resolution::PreserveChanged,
            false
        )
        .await
        .is_err()
    );
    assert!(
        release::resolve(
            &state,
            OWNER,
            &review.id,
            review.revision,
            observed.clone(),
            release::Resolution::RetryOriginal,
            false
        )
        .await
        .is_err()
    );
    // Demotion is checked live in the committing transaction, not from AuthUser.
    state
        .db
        .collection::<bson::Document>(USERS)
        .update_one(doc! {"_id":OWNER}, doc! {"$set":{"role_ids":[]}})
        .await
        .unwrap();
    assert!(
        release::resolve(
            &state,
            OWNER,
            &review.id,
            review.revision,
            observed.clone(),
            release::Resolution::PreserveChanged,
            false
        )
        .await
        .is_err()
    );
    admin(&state).await;
    release::resolve(
        &state,
        OWNER,
        &review.id,
        review.revision,
        observed.clone(),
        release::Resolution::PreserveChanged,
        false,
    )
    .await
    .unwrap();
    assert!(
        release::resolve(
            &state,
            OWNER,
            &review.id,
            review.revision,
            observed,
            release::Resolution::PreserveChanged,
            false
        )
        .await
        .is_err()
    );
    assert_eq!(current(&state, &review.id).await.expected_version, Some(1));
    assert_eq!(
        current(&state, &review.id).await.state,
        ReleaseState::Settled
    );
    assert!(!release::try_inline(&state, std::slice::from_ref(&review.id)).await);
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method != "DELETE")
    );
    upstream.abort();
    state.db.drop().await.unwrap();
    let (state, row, _, _, upstream) = fixture("release_admin_missing").await;
    admin(&state).await;
    let reserved = reserve(&state, &row).await;
    release::dispatch(&state.db, &reserved).await.unwrap();
    release::retire_attempt(&state, &reserved.id, ReleaseReason::SetupFailed)
        .await
        .unwrap();
    state
        .db
        .collection::<Release>(RELEASES)
        .update_one(
            doc! {"_id":&reserved.id},
            doc! {"$set":{"state":"review_required"}},
        )
        .await
        .unwrap();
    let review = current(&state, &reserved.id).await;
    assert!(matches!(
        release::resolve(
            &state,
            OWNER,
            &review.id,
            review.revision,
            release::Observation::Missing,
            release::Resolution::PreserveChanged,
            false
        )
        .await,
        Err(AppError::ValidationError(_))
    ));
    assert!(
        release::resolve(
            &state,
            OWNER,
            &review.id,
            review.revision,
            release::Observation::Missing,
            release::Resolution::ConfirmMissing,
            true
        )
        .await
        .is_err()
    );
    state.db.collection::<Release>(RELEASES).update_one(doc! {"_id":&reserved.id},doc! {"$set":{"dispatch_started_at":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::hours(25))}}).await.unwrap();
    assert!(
        release::resolve(
            &state,
            OWNER,
            &review.id,
            review.revision,
            release::Observation::Missing,
            release::Resolution::ConfirmMissing,
            false
        )
        .await
        .is_err()
    );
    release::resolve(
        &state,
        OWNER,
        &review.id,
        review.revision,
        release::Observation::Missing,
        release::Resolution::ConfirmMissing,
        true,
    )
    .await
    .unwrap();
    assert_eq!(
        current(&state, &review.id).await.resolution.as_deref(),
        Some("confirmed_missing")
    );
    assert!(release::try_inline(&state, std::slice::from_ref(&review.id)).await);
    upstream.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn continuity_newest_incompatible_predecessor_blocks_older_history_and_concurrent_successors()
{
    let (state, mut previous, _, _, upstream) = fixture("continuity_boundary").await;
    previous.status = "disconnected".into();
    previous.transport = "direct".into();
    previous.owner_sender_ids = vec!["older-owner".into()];
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&previous.id}, &previous)
        .await
        .unwrap();
    let mut boundary = previous.clone();
    boundary.id = Uuid::new_v4().to_string();
    boundary.created_at = Utc::now() + ChronoDuration::seconds(1);
    boundary.platform = "incompatible".into();
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .insert_one(&boundary)
        .await
        .unwrap();
    let next = direct_successor(&state, &previous).await;
    continuity::activate(
        &state.db,
        OWNER,
        &next.id,
        next.route_id.as_deref().unwrap(),
        None,
        true,
        "fresh",
    )
    .await
    .unwrap();
    assert!(
        load_channel(&state, OWNER, &next.id)
            .await
            .unwrap()
            .continuity_from_channel_id
            .is_none()
    );
    assert!(
        load_channel(&state, OWNER, &previous.id)
            .await
            .unwrap()
            .continuity_to_channel_id
            .is_none()
    );
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id":&next.id},
            doc! {"$set":{"status":"disconnected"}},
        )
        .await
        .unwrap();
    // Two contenders cannot both consume the explicitly selected predecessor.
    let one = direct_successor(&state, &previous).await;
    let two = direct_successor(&state, &previous).await;
    let (a, b) = tokio::join!(
        continuity::activate(
            &state.db,
            OWNER,
            &one.id,
            one.route_id.as_deref().unwrap(),
            Some(&previous.id),
            true,
            "one"
        ),
        continuity::activate(
            &state.db,
            OWNER,
            &two.id,
            two.route_id.as_deref().unwrap(),
            Some(&previous.id),
            true,
            "two"
        )
    );
    // A successor that lost the predecessor claim must not activate without it.
    assert_ne!(a.is_ok(), b.is_ok());
    upstream.abort();
    state.db.drop().await.unwrap();

    let (state, mut abandoned, _, _, upstream) = fixture("continuity_abandoned_source").await;
    abandoned.status = "disconnected".into();
    abandoned.route_id = None;
    abandoned.owner_sender_ids = vec!["uncommitted-owner".into()];
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&abandoned.id}, &abandoned)
        .await
        .unwrap();
    let next = direct_successor(&state, &abandoned).await;
    continuity::activate(
        &state.db,
        OWNER,
        &next.id,
        next.route_id.as_deref().unwrap(),
        Some(&abandoned.id),
        true,
        "fresh",
    )
    .await
    .unwrap();
    assert!(
        load_channel(&state, OWNER, &next.id)
            .await
            .unwrap()
            .continuity_from_channel_id
            .is_none()
    );
    assert!(
        load_channel(&state, OWNER, &abandoned.id)
            .await
            .unwrap()
            .continuity_to_channel_id
            .is_none()
    );
    upstream.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn release_conversion_uses_captured_or_exact_legacy_proof_without_a_second_owner() {
    for captured in [true, false] {
        let (state, mut row, raw, _, upstream) = fixture("release_conversion_proof").await;
        row.status = "disconnected".into();
        row.last_error = Some("platform_conversion".into());
        row.gateway_channel_id = Some("cmaeg1.ch.remote".into());
        row.gateway_record_id = Some(Uuid::new_v4().to_string());
        row.gateway_version = Some(3);
        row.binding_id = Some("converted-binding".into());
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .replace_one(doc! {"_id":&row.id}, &row)
            .await
            .unwrap();
        key_service::delete_api_key(&state.db, OWNER, row.agent_api_key_id.as_deref().unwrap())
            .await
            .unwrap();
        let task = crate::models::platform_bot_config::PlatformBotCleanup {
            id: Uuid::new_v4().to_string(),
            platform_config_id: Uuid::new_v4().to_string(),
            kind: "gateway".into(),
            resource_id: row.id.clone(),
            owner_user_id: OWNER.into(),
            capture_version: 1,
            turn_id: None,
            async_operation_ids: vec![],
            machine_jobs: vec![],
            machine_node_ids: vec![],
            gateway_channel_id: row.gateway_channel_id.clone(),
            gateway_version: row.gateway_version,
            record_id: if captured {
                row.gateway_record_id.clone()
            } else {
                None
            },
            binding_id: if captured {
                row.binding_id.clone()
            } else {
                None
            },
            agent_api_key_id: if captured {
                row.agent_api_key_id.clone()
            } else {
                None
            },
            completed: false,
            claim_id: None,
            retry_at: Utc::now(),
            created_at: Utc::now(),
        };
        state
            .db
            .collection::<crate::models::platform_bot_config::PlatformBotCleanup>(
                crate::models::platform_bot_config::CLEANUP_COLLECTION,
            )
            .insert_one(&task)
            .await
            .unwrap();
        assert_eq!(
            release::acknowledge_retired(&state, &raw, "converted-binding").await,
            release::RetiredAcknowledgement::Acknowledged
        );
        release::reconcile_step(&state).await.unwrap();
        assert_eq!(
            state
                .db
                .collection::<Release>(RELEASES)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        if !captured {
            let saved = state
                .db
                .collection::<crate::models::platform_bot_config::PlatformBotCleanup>(
                    crate::models::platform_bot_config::CLEANUP_COLLECTION,
                )
                .find_one(doc! {"_id":&task.id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(saved.agent_api_key_id, row.agent_api_key_id);
            assert_eq!(saved.binding_id, row.binding_id);
        }
        // Partially captured proof is not a legacy task and cannot be guessed.
        state
            .db
            .collection::<bson::Document>(crate::models::platform_bot_config::CLEANUP_COLLECTION)
            .update_one(
                doc! {"_id":&task.id},
                doc! {"$unset":{"agent_api_key_id":""}},
            )
            .await
            .unwrap();
        assert_eq!(
            release::acknowledge_retired(&state, &raw, "converted-binding").await,
            release::RetiredAcknowledgement::NotApplicable
        );
        upstream.abort();
        state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn release_database_failure_never_acknowledges_or_rolls_back_remote_state() {
    let (state, row, raw, server, upstream) = fixture("release_storage_failure").await;
    let reserved = reserve(&state, &row).await;
    release::dispatch(&state.db, &reserved).await.unwrap();
    let client = mongodb::Client::with_uri_str(
        "mongodb://127.0.0.1:1/?serverSelectionTimeoutMS=200&connectTimeoutMS=200",
    )
    .await
    .unwrap();
    let mut unavailable = state.clone();
    unavailable.db = client.database("unavailable");
    assert_eq!(
        release::acknowledge_retired(&unavailable, &raw, &reserved.binding_id).await,
        release::RetiredAcknowledgement::StorageUnavailable
    );
    assert!(
        release::retire_attempt(&unavailable, &reserved.id, ReleaseReason::SetupFailed)
            .await
            .is_err()
    );
    assert_eq!(
        current(&state, &reserved.id).await.state,
        ReleaseState::Provisional
    );
    assert!(server.received_requests().await.unwrap().is_empty());
    upstream.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn release_debug_admin_dto_and_audit_metadata_exclude_credential_material() {
    let (state, row, raw, _, upstream) = fixture("release_redaction").await;
    let reserved = reserve(&state, &row).await;
    release::dispatch(&state.db, &reserved).await.unwrap();
    release::retire_attempt(&state, &reserved.id, ReleaseReason::SetupFailed)
        .await
        .unwrap();
    let debug = format!("{reserved:?}");
    let dto = crate::handlers::nyxbot_gateway_releases::ReleaseResponse::from(
        current(&state, &reserved.id).await,
    );
    let serialized = serde_json::to_string(&dto).unwrap();
    let audit = state
        .db
        .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .find(doc! {"event_type":{"$regex":"^nyxbot_gateway_release"}})
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    let events = serde_json::to_string(&audit).unwrap();
    for value in [debug, serialized, events] {
        for secret in [&raw, &reserved.agent_api_key_id, &reserved.route_api_key_id] {
            assert!(!value.contains(secret));
        }
    }
    upstream.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn release_unknown_activation_reply_and_unavailable_reread_never_delete_adopted_channel() {
    let (state, row, _, server, upstream) = fixture("release_commit_unknown").await;
    let reserved = reserve(&state, &row).await;
    release::dispatch(&state.db, &reserved).await.unwrap();
    release::record_ack(&state.db, &reserved, "cmaeg1.ch.remote", 2)
        .await
        .unwrap();
    state.db.collection::<NyxbotChannel>(CHANNELS).update_one(doc! {"_id":&row.id},doc! {"$set":{"gateway_channel_id":"cmaeg1.ch.remote","gateway_record_id":&reserved.record_id,"gateway_version":2_i64}}).await.unwrap();
    continuity::activate(
        &state.db,
        OWNER,
        &row.id,
        row.route_id.as_deref().unwrap(),
        None,
        true,
        "committed-link",
    )
    .await
    .unwrap();
    // The caller loses the commit response and cannot establish local outcome.
    let client = mongodb::Client::with_uri_str(
        "mongodb://127.0.0.1:1/?serverSelectionTimeoutMS=200&connectTimeoutMS=200",
    )
    .await
    .unwrap();
    let mut unavailable = state.clone();
    unavailable.db = client.database("unavailable");
    assert!(load_channel(&unavailable, OWNER, &row.id).await.is_err());
    assert!(
        release::retire_attempt(&unavailable, &reserved.id, ReleaseReason::SetupFailed)
            .await
            .is_err()
    );
    // Once reads resume, an old setup rollback has no authority over adoption.
    release::retire_attempt(&state, &reserved.id, ReleaseReason::SetupFailed)
        .await
        .unwrap();
    release::retire_connection(
        &state,
        &row,
        ReleaseReason::SetupFailed,
        "failed",
        Some("gateway_unavailable"),
    )
    .await
    .unwrap();
    assert_eq!(
        load_channel(&state, OWNER, &row.id).await.unwrap().status,
        "active"
    );
    assert!(release::get(&state.db, &reserved.id).await.is_err());
    assert!(server.received_requests().await.unwrap().is_empty());
    upstream.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn release_missing_proof_is_review_only_and_rotated_predecessors_never_acknowledge() {
    let (state, mut row, raw, server, upstream) = fixture("release_missing_proof").await;
    row.status = "disconnected".into();
    row.gateway_channel_id = Some("cmaeg1.ch.remote".into());
    row.gateway_record_id = Some(Uuid::new_v4().to_string());
    row.binding_id = None;
    row.gateway_version = None;
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&row.id}, &row)
        .await
        .unwrap();
    use tracing::instrument::WithSubscriber;
    let output = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let log_output = output.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::ERROR)
        .with_writer(move || CapturedReleaseLog(log_output.clone()))
        .finish();
    release::reconcile_step(&state)
        .with_subscriber(subscriber)
        .await
        .unwrap();
    let logged = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    assert!(logged.contains("retirement_proof_missing"));
    for secret in [
        &raw,
        row.agent_api_key_id.as_ref().unwrap(),
        &row.route_api_key_id,
    ] {
        assert!(!logged.contains(secret));
    }
    let review = state
        .db
        .collection::<Release>(RELEASES)
        .find_one(doc! {"channel_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(review.state, ReleaseState::ReviewRequired);
    assert_eq!(
        review.last_error.as_deref(),
        Some("retirement_proof_missing")
    );
    assert_eq!(
        release::acknowledge_retired(&state, &raw, "arbitrary").await,
        release::RetiredAcknowledgement::NotApplicable
    );
    state
        .db
        .collection::<Release>(RELEASES)
        .update_one(
            doc! {"_id":&review.id},
            doc! {"$set":{"binding_id":"retired-binding"}},
        )
        .await
        .unwrap();
    let successor = key(&state, "rotation successor").await;
    state
        .db
        .collection::<bson::Document>(KEYS)
        .update_one(
            doc! {"_id":&successor.id},
            doc! {"$set":{"rotation_predecessor_id":&review.agent_api_key_id}},
        )
        .await
        .unwrap();
    assert_eq!(
        release::acknowledge_retired(&state, &raw, "retired-binding").await,
        release::RetiredAcknowledgement::NotApplicable
    );
    assert!(server.received_requests().await.unwrap().is_empty());
    upstream.abort();
    state.db.drop().await.unwrap();

    let (state, mut row, _, server, upstream) = fixture("release_missing_version").await;
    row.status = "disconnected".into();
    row.gateway_channel_id = Some("cmaeg1.ch.remote".into());
    row.gateway_record_id = Some(Uuid::new_v4().to_string());
    row.binding_id = Some("known-binding".into());
    row.gateway_version = None;
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&row.id}, &row)
        .await
        .unwrap();
    let ids = release::retire_connection(
        &state,
        &row,
        ReleaseReason::Disconnect,
        "disconnected",
        None,
    )
    .await
    .unwrap();
    let review = current(&state, &ids[0]).await;
    assert_eq!(review.state, ReleaseState::ReviewRequired);
    mount_remote(&server, &review, 7).await;
    let observed = release::observe(&state, &review).await.unwrap();
    assert_eq!(observed, release::Observation::Malformed);
    admin(&state).await;
    assert!(matches!(
        release::resolve(
            &state,
            OWNER,
            &review.id,
            review.revision,
            observed,
            release::Resolution::RetryOriginal,
            false,
        )
        .await,
        Err(AppError::ValidationError(_))
    ));
    // Even an erroneously requeued row must not adopt the observed version.
    state
        .db
        .collection::<Release>(RELEASES)
        .update_one(doc! {"_id":&review.id}, doc! {"$set":{"state":"due"}})
        .await
        .unwrap();
    assert!(!execute(&state, &review.id).await);
    let review = current(&state, &review.id).await;
    assert_eq!(review.state, ReleaseState::ReviewRequired);
    assert_eq!(review.expected_version, None);
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|request| request.method != "DELETE")
    );
    upstream.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn release_concurrent_activation_and_expiry_reclamation_commit_one_outcome() {
    for expired in [false, true] {
        let (state, row, _, _, upstream) = fixture("release_activation_race").await;
        let reserved = reserve(&state, &row).await;
        release::dispatch(&state.db, &reserved).await.unwrap();
        release::record_ack(&state.db, &reserved, "cmaeg1.ch.remote", 2)
            .await
            .unwrap();
        state.db.collection::<NyxbotChannel>(CHANNELS).update_one(doc! {"_id":&row.id},doc! {"$set":{"gateway_channel_id":"cmaeg1.ch.remote","gateway_record_id":&reserved.record_id,"gateway_version":2_i64}}).await.unwrap();
        if expired {
            expire(&state, &reserved.id).await;
        }
        let (activation, reclaimed) = tokio::join!(
            continuity::activate(
                &state.db,
                OWNER,
                &row.id,
                row.route_id.as_deref().unwrap(),
                None,
                true,
                "racing-link"
            ),
            release::retire_attempt(&state, &reserved.id, ReleaseReason::StaleSetup)
        );
        reclaimed.unwrap();
        assert_eq!(activation.is_ok(), !expired);
        let row = load_channel(&state, OWNER, &row.id).await.unwrap();
        assert_eq!(row.status == "active", !expired);
        let obligation = release::get(&state.db, &reserved.id).await.ok();
        assert_eq!(
            obligation
                .as_ref()
                .is_some_and(|r| r.state == ReleaseState::Due),
            expired
        );
        upstream.abort();
        state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn continuity_explicit_reconnect_returns_a_fresh_owner_link_and_completed_lineage() {
    let (state, _, upstream) = setup("continuity_connect_api").await;
    let bot = bot_doc("lark", "Reconnect bot");
    let bot_id = bot.get_str("_id").unwrap().to_owned();
    state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .insert_one(bot)
        .await
        .unwrap();
    let agent = crate::services::assistant_team_service::ensure_nyxbot(&state.db, OWNER)
        .await
        .unwrap();
    let (before, first) = Box::pin(connect(&state, OWNER, None, &bot_id, &agent))
        .await
        .unwrap();
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id":&before.id},
            doc! {"$set":{"owner_sender_ids":["verified-person"],"private_chats":"everyone"}},
        )
        .await
        .unwrap();
    disconnect(&state, OWNER, &before.id).await.unwrap();
    let (after, second) = Box::pin(connect(&state, OWNER, None, &bot_id, &agent))
        .await
        .unwrap();
    assert_ne!(first.code.as_str(), second.code.as_str());
    assert!(second.code.starts_with("nyxlink_"));
    assert_eq!(
        after.link_code_hash.as_deref(),
        Some(sha256_hex(second.code.as_bytes()).as_str())
    );
    assert_eq!(after.owner_sender_ids, ["verified-person"]);
    assert_eq!(after.private_chats.as_deref(), Some("everyone"));
    assert_eq!(
        after.continuity_from_channel_id.as_deref(),
        Some(before.id.as_str())
    );
    assert_eq!(
        load_channel(&state, OWNER, &before.id)
            .await
            .unwrap()
            .continuity_to_channel_id
            .as_deref(),
        Some(after.id.as_str())
    );
    upstream.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn continuity_activation_policy_sync_survives_a_crash_before_inline_sync() {
    let (state, row, _, server, upstream) = fixture("continuity_policy_retry").await;
    let reserved = reserve(&state, &row).await;
    release::dispatch(&state.db, &reserved).await.unwrap();
    release::record_ack(&state.db, &reserved, "cmaeg1.ch.remote", 2)
        .await
        .unwrap();
    state.db.collection::<NyxbotChannel>(CHANNELS).update_one(doc! {"_id":&row.id},
        doc! {"$set":{"gateway_channel_id":"cmaeg1.ch.remote","gateway_record_id":&reserved.record_id,"gateway_version":2_i64}}).await.unwrap();
    continuity::activate(
        &state.db,
        OWNER,
        &row.id,
        row.route_id.as_deref().unwrap(),
        None,
        true,
        "crash-safe-link",
    )
    .await
    .unwrap();
    let active = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert!(
        active
            .gateway_groups_retry_at
            .is_some_and(|at| at <= Utc::now())
    );
    assert!(server.received_requests().await.unwrap().is_empty());
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(remote(&reserved, 2)))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/channels/cmaeg1.ch.remote"))
        .respond_with(|request: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            assert_eq!(body["expected_version"], 2);
            ResponseTemplate::new(200).set_body_json(json!({"version":3,"definition":body}))
        })
        .expect(1)
        .mount(&server)
        .await;
    // Use the existing sweep, with no inline activation sync having run.
    check_deliveries(&state).await.unwrap();
    let synced = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert_eq!(synced.status, "active");
    assert_eq!(synced.gateway_version, Some(3));
    assert!(synced.gateway_no_progress);
    assert!(synced.gateway_groups_retry_at.is_none());
    assert_eq!(synced.link_code_hash.as_deref(), Some("crash-safe-link"));
    upstream.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn release_admin_retry_requires_the_fresh_original_observation() {
    let (state, row, _, server, upstream) = fixture("release_admin_retry").await;
    admin(&state).await;
    let reserved = reserve(&state, &row).await;
    release::dispatch(&state.db, &reserved).await.unwrap();
    release::record_ack(&state.db, &reserved, "cmaeg1.ch.remote", 1)
        .await
        .unwrap();
    release::retire_attempt(&state, &reserved.id, ReleaseReason::SetupFailed)
        .await
        .unwrap();
    state
        .db
        .collection::<Release>(RELEASES)
        .update_one(
            doc! {"_id":&reserved.id},
            doc! {"$set":{"state":"review_required"}},
        )
        .await
        .unwrap();
    let review = current(&state, &reserved.id).await;
    mount_remote(&server, &reserved, 1).await;
    let inspected = release::observe(&state, &review).await.unwrap();
    assert!(matches!(
        release::resolve(
            &state,
            OWNER,
            &review.id,
            review.revision,
            inspected.clone(),
            release::Resolution::PreserveChanged,
            false
        )
        .await,
        Err(AppError::ValidationError(_))
    ));
    server.reset().await;
    mount_remote(&server, &reserved, 2).await;
    assert!(matches!(
        release::resolve(
            &state,
            OWNER,
            &review.id,
            review.revision,
            inspected.clone(),
            release::Resolution::RetryOriginal,
            false
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    assert_eq!(
        current(&state, &review.id).await.state,
        ReleaseState::ReviewRequired
    );
    server.reset().await;
    mount_remote(&server, &reserved, 1).await;
    release::resolve(
        &state,
        OWNER,
        &review.id,
        review.revision,
        inspected,
        release::Resolution::RetryOriginal,
        false,
    )
    .await
    .unwrap();
    let due = current(&state, &review.id).await;
    assert_eq!(due.state, ReleaseState::Due);
    assert_eq!(due.expected_version, Some(1));
    let claimed = release::claim(&state.db, Some(&due.id))
        .await
        .unwrap()
        .unwrap();
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method != "DELETE")
    );
    server.reset().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;
    let error = crate::handlers::nyxbot_gateway_releases::inspect(
        State(state.clone()),
        crate::test_utils::test_auth_user(OWNER),
        Path(due.id.clone()),
    )
    .await
    .err()
    .unwrap();
    assert!(matches!(error, AppError::OAuthAuthorityUnavailable));
    server.reset().await;
    mount_remote(&server, &reserved, 1).await;
    Mock::given(method("DELETE"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    assert!(release::execute(&state, claimed).await.unwrap());
    assert_eq!(current(&state, &due.id).await.state, ReleaseState::Settled);
    upstream.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn release_retired_parent_never_acknowledges_a_child_credential() {
    use crate::models::api_key_credential::{ApiKeyCredential, ApiKeyCredentialKind};
    use crate::services::api_key_credential_service as credentials;
    let (state, row, raw, _, upstream) = fixture("release_child_credential").await;
    let reserved = reserve(&state, &row).await;
    let child = credentials::generate_secret();
    let child_row = ApiKeyCredential {
        id: Uuid::new_v4().to_string(),
        api_key_id: reserved.agent_api_key_id.clone(),
        user_id: OWNER.into(),
        secret_hash: crate::crypto::token::hash_token(&child),
        secret_prefix: crate::crypto::token::agent_key_display_prefix(&child)
            .unwrap()
            .into(),
        kind: ApiKeyCredentialKind::AgentKeyLogin,
        label: "child".into(),
        login_request_id: Uuid::new_v4().to_string(),
        is_active: true,
        revoked_at: None,
        revoked_reason: None,
        expires_at: None,
        last_used_at: None,
        created_at: Utc::now(),
    };
    state
        .db
        .collection::<ApiKeyCredential>(crate::models::api_key_credential::COLLECTION_NAME)
        .insert_one(&child_row)
        .await
        .unwrap();
    release::dispatch(&state.db, &reserved).await.unwrap();
    release::retire_attempt(&state, &reserved.id, ReleaseReason::SetupFailed)
        .await
        .unwrap();
    let revoked = state
        .db
        .collection::<ApiKeyCredential>(crate::models::api_key_credential::COLLECTION_NAME)
        .find_one(doc! {"_id":&child_row.id})
        .await
        .unwrap()
        .unwrap();
    assert!(!revoked.is_active);
    assert_eq!(
        release::acknowledge_retired(&state, &child, &reserved.binding_id).await,
        release::RetiredAcknowledgement::NotApplicable
    );
    assert_eq!(
        delete_binding(
            State(state.clone()),
            Path(reserved.binding_id.clone()),
            bearer(&child)
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        release::acknowledge_retired(&state, &raw, &reserved.binding_id).await,
        release::RetiredAcknowledgement::Acknowledged
    );
    upstream.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn release_delete_authority_refusal_escalates_without_changing_authority() {
    let (state, row, _, server, upstream) = fixture("release_delete_auth").await;
    let reserved = reserve(&state, &row).await;
    release::dispatch(&state.db, &reserved).await.unwrap();
    release::record_ack(&state.db, &reserved, "cmaeg1.ch.remote", 1)
        .await
        .unwrap();
    release::retire_attempt(&state, &reserved.id, ReleaseReason::SetupFailed)
        .await
        .unwrap();
    mount_remote(&server, &reserved, 1).await;
    Mock::given(method("DELETE"))
        .and(path("/channels/cmaeg1.ch.remote"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;
    assert!(!execute(&state, &reserved.id).await);
    assert_eq!(current(&state, &reserved.id).await.state, ReleaseState::Due);
    state.db.collection::<Release>(RELEASES).update_one(doc! {"_id":&reserved.id},
        doc! {"$set":{"due_at":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::hours(25))}}).await.unwrap();
    assert!(!execute(&state, &reserved.id).await);
    let review = current(&state, &reserved.id).await;
    assert_eq!(review.state, ReleaseState::ReviewRequired);
    assert_eq!(
        review.last_error.as_deref(),
        Some("gateway_authority_refused")
    );
    assert_eq!(review.expected_version, Some(1));
    upstream.abort();
    state.db.drop().await.unwrap();
}

struct CapturedReleaseLog(Arc<std::sync::Mutex<Vec<u8>>>);
impl std::io::Write for CapturedReleaseLog {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn release_stale_gateway_move_keeps_the_working_direct_route() {
    let (state, mut row, _, server, upstream) = fixture("release_pending_move").await;
    row.status = "active".into();
    row.transport = "direct".into();
    row.agent_api_key_id = None;
    row.agent_key_ciphertext = None;
    row.link_code_hash = Some(sha256_hex(b"working direct owner link"));
    row.link_code_expires_at = Some(
        bson::DateTime::from_chrono(Utc::now() + ChronoDuration::hours(LINK_CODE_TTL_HOURS))
            .to_chrono(),
    );
    let agent = key(&state, "pending gateway agent").await;
    let route = key(&state, "pending gateway route").await;
    row.pending_agent_api_key_id = Some(agent.id.clone());
    row.pending_route_api_key_id = Some(route.id.clone());
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&row.id}, &row)
        .await
        .unwrap();
    let mut attempt = row.clone();
    attempt.agent_api_key_id = Some(agent.id.clone());
    attempt.route_api_key_id = route.id.clone();
    let reserved = release::reserve(
        &state,
        &attempt,
        doc! {"_id":&row.id,"status":"active","transport":"direct",
        "pending_agent_api_key_id":&agent.id,"pending_route_api_key_id":&route.id},
    )
    .await
    .unwrap();
    release::dispatch(&state.db, &reserved).await.unwrap();
    release::record_ack(&state.db, &reserved, "cmaeg1.ch.remote", 1)
        .await
        .unwrap();
    let provisional = current(&state, &reserved.id).await;
    let deadline = provisional.provisional_expires_at.unwrap();
    assert_eq!(
        deadline - provisional.created_at,
        ChronoDuration::minutes(release::SETUP_MINUTES)
    );
    assert!(transport::stale_move_cutoff(deadline).to_chrono() <= provisional.created_at);
    state.db.collection::<NyxbotChannel>(CHANNELS).update_one(doc! {"_id":&row.id},
        doc! {"$set":{"gateway_attempted_at":bson::DateTime::from_chrono(provisional.created_at)}}).await.unwrap();
    switch_to_gateway(&state).await.unwrap();
    assert_eq!(
        current(&state, &reserved.id).await.state,
        ReleaseState::Provisional
    );
    assert_eq!(
        load_channel(&state, OWNER, &row.id)
            .await
            .unwrap()
            .pending_agent_api_key_id
            .as_deref(),
        Some(agent.id.as_str())
    );
    let started = Utc::now() - ChronoDuration::minutes(release::SETUP_MINUTES + 1);
    state.db.collection::<Release>(RELEASES).update_one(doc! {"_id":&reserved.id},
        doc! {"$set":{"provisional_expires_at":bson::DateTime::from_chrono(started+ChronoDuration::minutes(release::SETUP_MINUTES))}}).await.unwrap();
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"gateway_attempted_at":bson::DateTime::from_chrono(started)}},
        )
        .await
        .unwrap();
    switch_to_gateway(&state).await.unwrap();
    let direct = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert_eq!(direct.status, "active");
    assert_eq!(direct.transport, "direct");
    assert_eq!(direct.route_api_key_id, row.route_api_key_id);
    assert_eq!(direct.route_id, row.route_id);
    assert_eq!(direct.link_code_hash, row.link_code_hash);
    assert_eq!(direct.link_code_expires_at, row.link_code_expires_at);
    assert!(direct.gateway_setup_release_id.is_none());
    assert!(direct.pending_agent_api_key_id.is_none() && direct.pending_route_api_key_id.is_none());
    assert!(
        key_service::get_api_key(&state.db, OWNER, &row.route_api_key_id)
            .await
            .unwrap()
            .is_active
    );
    let active_route = state
        .db
        .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
        .find_one(
            doc! {"_id":&row.route_id,"is_active":true,"agent_api_key_id":&row.route_api_key_id},
        )
        .await
        .unwrap();
    assert!(active_route.is_some());
    for id in [&agent.id, &route.id] {
        assert_retired_key(&state, id).await;
    }
    let due = current(&state, &reserved.id).await;
    assert_eq!(due.state, ReleaseState::Due);
    assert_eq!(due.gateway_channel_id.as_deref(), Some("cmaeg1.ch.remote"));
    assert_eq!(due.expected_version, Some(1));
    assert!(server.received_requests().await.unwrap().is_empty());
    upstream.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn release_sweep_bounds_provisional_retirement_to_ten_per_pass() {
    let (state, row, _, server, upstream) = fixture("release_sweep_bound").await;
    let reserved = reserve(&state, &row).await;
    expire(&state, &reserved.id).await;
    for _ in 0..10 {
        let (mut row, _) = channel(&state, "gateway").await;
        row.status = "pending".into();
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .replace_one(doc! {"_id":&row.id}, &row)
            .await
            .unwrap();
        let reserved = reserve(&state, &row).await;
        expire(&state, &reserved.id).await;
    }
    release::sweep(&state).await.unwrap();
    assert_eq!(
        state
            .db
            .collection::<Release>(RELEASES)
            .count_documents(doc! {"state":"settled"})
            .await
            .unwrap(),
        10
    );
    assert_eq!(
        state
            .db
            .collection::<Release>(RELEASES)
            .count_documents(doc! {"state":"provisional"})
            .await
            .unwrap(),
        1
    );
    release::sweep(&state).await.unwrap();
    assert_eq!(
        state
            .db
            .collection::<Release>(RELEASES)
            .count_documents(doc! {"state":"settled"})
            .await
            .unwrap(),
        11
    );
    assert!(server.received_requests().await.unwrap().is_empty());
    upstream.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn continuity_existing_target_keeps_conflicts_and_moves_only_matching_metadata() {
    let (state, _, upstream) = setup("continuity_target_wins").await;
    let bot = bot_doc("lark", "Existing connection");
    let bot_id = bot.get_str("_id").unwrap().to_owned();
    state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .insert_one(bot)
        .await
        .unwrap();
    let agent = crate::services::assistant_team_service::ensure_nyxbot(&state.db, OWNER)
        .await
        .unwrap();
    let (previous, _) = Box::pin(connect(&state, OWNER, None, &bot_id, &agent))
        .await
        .unwrap();
    disconnect(&state, OWNER, &previous.id).await.unwrap();
    let (target, old_link) = Box::pin(connect(&state, OWNER, None, &bot_id, &agent))
        .await
        .unwrap();
    // Simulate a healthy reconnect written by an older replica, with both
    // generations already owning history for the same stable partition.
    let channels = state.db.collection::<NyxbotChannel>(CHANNELS);
    channels
        .update_one(
            doc! {"_id":&previous.id},
            doc! {"$unset":{"continuity_to_channel_id":""}},
        )
        .await
        .unwrap();
    channels
        .update_one(
            doc! {"_id":&target.id},
            doc! {"$unset":{"continuity_from_channel_id":"","continuity_restored_at":""}},
        )
        .await
        .unwrap();
    let conflicting = thread_doc(&state, &previous, "chat_conflict", false).await;
    let moved = thread_doc(&state, &previous, "chat_move", false).await;
    let opaque = thread_doc(&state, &previous, PARTITION, false).await;
    let retained = thread_doc(&state, &target, "chat_conflict", false).await;
    let threads = state.db.collection::<bson::Document>(THREADS);
    threads
        .update_one(
            doc! {"_id":&retained},
            doc! {"$set":{"conversation_id":"target-conversation"}},
        )
        .await
        .unwrap();
    let target_before = threads
        .find_one(doc! {"_id":&retained})
        .await
        .unwrap()
        .unwrap();
    let conversations = state.db.collection::<bson::Document>(CONVERSATIONS);
    for (id, channel_partition, reply_partition) in [
        ("conflict", "chat_conflict", "chat_conflict"),
        ("moved", "chat_move", "chat_move"),
        ("mixed", "chat_conflict", "chat_move"),
        ("opaque", PARTITION, PARTITION),
        ("no-thread", "chat_absent", "chat_absent"),
    ] {
        conversations
            .insert_one(doc! {"_id":id,"user_id":OWNER,
            "channel":{"nyxbot_channel_id":&previous.id,"partition":channel_partition},
            "reply_channel":{"nyxbot_channel_id":&previous.id,"partition":reply_partition}})
            .await
            .unwrap();
    }
    let audit = state.db.collection::<crate::models::audit_log::AuditLog>(
        crate::models::audit_log::COLLECTION_NAME,
    );
    let filter =
        doc! {"event_type":"nyxbot_channel_continuity_restored","event_data.channel_id":&target.id};
    let events_before = audit.count_documents(filter.clone()).await.unwrap();
    let (restored, link) = Box::pin(connect(&state, OWNER, None, &bot_id, &agent))
        .await
        .unwrap();
    assert_eq!(restored.id, target.id);
    assert_eq!(restored.status, "active");
    assert_ne!(link.code.as_str(), old_link.code.as_str());
    assert_eq!(
        restored.link_code_hash.as_deref(),
        Some(sha256_hex(link.code.as_bytes()).as_str())
    );
    assert!(link.code.starts_with("nyxlink_"));
    assert_eq!(
        restored.continuity_from_channel_id.as_deref(),
        Some(previous.id.as_str())
    );
    for (id, channel_id) in [
        (&conflicting, &previous.id),
        (&moved, &target.id),
        (&opaque, &previous.id),
    ] {
        assert_eq!(
            threads
                .find_one(doc! {"_id":id})
                .await
                .unwrap()
                .unwrap()
                .get_str("channel_id")
                .unwrap(),
            channel_id
        );
    }
    assert_eq!(
        threads
            .find_one(doc! {"_id":&retained})
            .await
            .unwrap()
            .unwrap(),
        target_before
    );
    for id in ["conflict", "moved", "mixed", "opaque", "no-thread"] {
        let document = conversations
            .find_one(doc! {"_id":id})
            .await
            .unwrap()
            .unwrap();
        for field in ["channel", "reply_channel"] {
            let metadata = document.get_document(field).unwrap();
            let expected = if metadata.get_str("partition").unwrap() == "chat_move" {
                &target.id
            } else {
                &previous.id
            };
            assert_eq!(metadata.get_str("nyxbot_channel_id").unwrap(), expected);
        }
    }
    assert_eq!(
        audit.count_documents(filter.clone()).await.unwrap(),
        events_before + 1
    );
    let event = audit
        .find_one(filter.clone())
        .sort(doc! {"seq":-1})
        .await
        .unwrap()
        .unwrap()
        .event_data
        .unwrap();
    assert_eq!(event["moved"], 1);
    assert_eq!(event["kept_conflicting"], 1);
    assert_eq!(
        load_channel(&state, OWNER, &previous.id)
            .await
            .unwrap()
            .continuity_to_channel_id
            .as_deref(),
        Some(target.id.as_str())
    );
    let (again, fresh_link) = Box::pin(connect(&state, OWNER, None, &bot_id, &agent))
        .await
        .unwrap();
    assert_eq!(again.id, target.id);
    assert_ne!(fresh_link.code.as_str(), link.code.as_str());
    assert_eq!(
        audit.count_documents(filter).await.unwrap(),
        events_before + 1
    );
    assert_eq!(
        threads
            .find_one(doc! {"_id":retained})
            .await
            .unwrap()
            .unwrap(),
        target_before
    );
    upstream.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn release_admin_preserves_every_nonmatching_nonmissing_observation_without_deleting() {
    for (status, expected) in [
        (
            "changed",
            release::Observation::Changed { version: Some(2) },
        ),
        ("live_reference", release::Observation::LiveReference),
        (
            "destination_changed",
            release::Observation::DestinationChanged,
        ),
        ("owner_unavailable", release::Observation::OwnerUnavailable),
        ("malformed", release::Observation::Malformed),
    ] {
        let (state, row, _, server, upstream) = fixture("release_preserve_status").await;
        admin(&state).await;
        let actor = Uuid::new_v4().to_string();
        let mut user = test_user(&actor, UserType::Person);
        user.role_ids = vec![
            crate::services::role_service::get_platform_role_ids(&state.db)
                .await
                .unwrap()
                .admin,
        ];
        state
            .db
            .collection::<crate::models::user::User>(USERS)
            .insert_one(user)
            .await
            .unwrap();
        let reserved = reserve(&state, &row).await;
        release::dispatch(&state.db, &reserved).await.unwrap();
        release::record_ack(&state.db, &reserved, "cmaeg1.ch.remote", 1)
            .await
            .unwrap();
        release::retire_attempt(&state, &reserved.id, ReleaseReason::SetupFailed)
            .await
            .unwrap();
        let mut body = remote(&reserved, if status == "changed" { 2 } else { 1 });
        match status {
            "live_reference" => {
                state
                    .db
                    .collection::<NyxbotChannel>(CHANNELS)
                    .update_one(doc! {"_id":&row.id}, doc! {"$set":{"status":"active"}})
                    .await
                    .unwrap();
            }
            "destination_changed" => {
                state
                    .db
                    .collection::<DownstreamService>(SERVICES)
                    .update_one(
                        doc! {"slug":"cmaeg"},
                        doc! {"$set":{"base_url":format!("{}/changed",server.uri())}},
                    )
                    .await
                    .unwrap();
            }
            "owner_unavailable" => {
                state
                    .db
                    .collection::<bson::Document>(USERS)
                    .update_one(doc! {"_id":OWNER}, doc! {"$set":{"is_active":false}})
                    .await
                    .unwrap();
            }
            "malformed" => body["provider"]["binding_id"] = json!("wrong-binding"),
            _ => {}
        }
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        assert!(!execute(&state, &reserved.id).await);
        let review = current(&state, &reserved.id).await;
        assert_eq!(review.state, ReleaseState::ReviewRequired);
        let observation = release::observe(&state, &review).await.unwrap();
        assert_eq!(observation, expected);
        for action in [
            release::Resolution::RetryOriginal,
            release::Resolution::ConfirmMissing,
        ] {
            assert!(matches!(
                release::resolve(
                    &state,
                    &actor,
                    &review.id,
                    review.revision,
                    observation.clone(),
                    action,
                    false
                )
                .await,
                Err(AppError::ValidationError(_))
            ));
        }
        release::resolve(
            &state,
            &actor,
            &review.id,
            review.revision,
            observation,
            release::Resolution::PreserveChanged,
            false,
        )
        .await
        .unwrap();
        let settled = current(&state, &review.id).await;
        assert_eq!(settled.state, ReleaseState::Settled);
        assert_eq!(settled.resolution.as_deref(), Some("preserved"));
        assert_eq!(settled.expected_version, Some(1));
        assert!(
            release::claim(&state.db, Some(&review.id))
                .await
                .unwrap()
                .is_none()
        );
        assert!(!release::try_inline(&state, std::slice::from_ref(&review.id)).await);
        let events: Vec<crate::models::audit_log::AuditLog> = state.db.collection(crate::models::audit_log::COLLECTION_NAME)
            .find(doc! {"event_type":{"$in":["nyxbot_gateway_release_reviewed","nyxbot_gateway_release_settled"]},"event_data.obligation_id":&review.id})
            .await.unwrap().try_collect().await.unwrap();
        assert_eq!(events.len(), 2);
        for event in events {
            let data = event.event_data.unwrap();
            assert_eq!(data["resolution"], "preserved");
            assert_eq!(data["observation_status"], status);
        }
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|request| request.method != "DELETE")
        );
        upstream.abort();
        state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn release_channel_ids_are_opaque_and_reject_empty_oversized_or_whitespace_values() {
    let (state, row, _, server, upstream) = fixture("release_opaque_id").await;
    let reserved = reserve(&state, &row).await;
    release::dispatch(&state.db, &reserved).await.unwrap();
    release::retire_attempt(&state, &reserved.id, ReleaseReason::SetupFailed)
        .await
        .unwrap();
    let retired = current(&state, &reserved.id).await;
    for (id, valid) in [
        ("cmaeg2.ch.remote".to_owned(), true),
        ("opaque/v2?channel=one".to_owned(), true),
        ("a".repeat(512), true),
        (String::new(), false),
        ("a".repeat(513), false),
        (" remote".to_owned(), false),
        ("remote\n".to_owned(), false),
        ("remote\0".to_owned(), false),
        ("remote\u{0085}".to_owned(), false),
        ("remote\u{2003}".to_owned(), false),
    ] {
        server.reset().await;
        let mut body = remote(&reserved, 1);
        body["channel_id"] = json!(id);
        body["endpoints"] = json!({"nyxid_callback_url":format!("{}/callback", server.uri())});
        let created = release::validate_create(&reserved, &body);
        assert_eq!(created.is_ok(), valid);
        if valid {
            assert_eq!(created.unwrap().0, id);
        }
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        assert_eq!(
            release::observe(&state, &retired).await.unwrap(),
            if valid {
                release::Observation::Matching {
                    channel_id: id,
                    version: 1,
                }
            } else {
                release::Observation::Malformed
            }
        );
    }
    upstream.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn continuity_local_activation_failure_reports_setup_interrupted_and_retires_route() {
    let (state, _, upstream) = setup("continuity_interrupted").await;
    let bot = bot_doc("lark", "Interrupted setup");
    let bot_id = bot.get_str("_id").unwrap().to_owned();
    state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .insert_one(bot)
        .await
        .unwrap();
    let agent = crate::services::assistant_team_service::ensure_nyxbot(&state.db, OWNER)
        .await
        .unwrap();
    state
        .db
        .run_command(doc! {"collMod":CHANNELS,"validator":{"status":{"$ne":"active"}}})
        .await
        .unwrap();
    assert!(
        Box::pin(connect(&state, OWNER, None, &bot_id, &agent))
            .await
            .is_err()
    );
    let failed = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"channel_bot_id":&bot_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.status, "failed");
    assert_eq!(failed.last_error.as_deref(), Some("setup_interrupted"));
    assert!(failed.link_code_hash.is_none());
    assert_retired_key(&state, &failed.route_api_key_id).await;
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
            .count_documents(doc! {"channel_bot_id":&bot_id,"is_active":true})
            .await
            .unwrap(),
        0
    );
    upstream.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn domain_alias_saved_gateway_issuer_accepts_overlap_but_never_destination_drift() {
    let (mut state, row, _, server, upstream) = fixture("domain_alias_release").await;
    state.config.jwt_issuer = "https://old.example".into();
    let reserved = reserve(&state, &row).await;
    release::dispatch(&state.db, &reserved).await.unwrap();
    release::retire_attempt(&state, &reserved.id, ReleaseReason::SetupFailed)
        .await
        .unwrap();
    mount_remote(&server, &reserved, 1).await;
    state.config.jwt_issuer = "https://new.example".into();
    assert_eq!(
        release::observe(&state, &reserved).await.unwrap(),
        release::Observation::DestinationChanged
    );
    state.config.domain_aliases.base_urls = vec!["https://old.example".into()];
    assert!(matches!(
        release::observe(&state, &reserved).await.unwrap(),
        release::Observation::Matching { .. }
    ));
    state
        .db
        .collection::<DownstreamService>(SERVICES)
        .update_one(
            doc! {"slug":"cmaeg"},
            doc! {"$set":{"base_url":"https://different-gateway.example"}},
        )
        .await
        .unwrap();
    assert_eq!(
        release::observe(&state, &reserved).await.unwrap(),
        release::Observation::DestinationChanged
    );
    upstream.abort();
}
