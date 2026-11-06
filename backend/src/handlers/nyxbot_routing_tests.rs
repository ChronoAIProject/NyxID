//! Wire contract pinned to CMAEG's deployed OpenAPI and protocol, not the
//! desired-policy builder. Only synthetic identities/credentials are used.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Default)]
pub(super) struct RoutingBehavior {
    pub conflict: AtomicBool,
    pub unacknowledged: AtomicBool,
    pub wrong_version: AtomicBool,
    pub lose_response: AtomicBool,
    pub pause: AtomicBool,
    pub arrived: tokio::sync::Notify,
    pub resume: tokio::sync::Notify,
}

pub(super) fn request_matches_schema(name: &str, body: &Value) -> bool {
    let schemas: Value =
        serde_json::from_str(include_str!("fixtures/cmaeg-channel-request-schemas.json")).unwrap();
    let schema = &schemas["schemas"][name];
    assert_eq!(schema["additionalProperties"], false);
    let Some(fields) = body.as_object() else {
        return false;
    };
    let properties = schema["properties"].as_object().unwrap();
    schema["required"]
        .as_array()
        .unwrap()
        .iter()
        .all(|key| fields.contains_key(key.as_str().unwrap()))
        && fields.iter().all(|(key, value)| {
            let Some(property) = properties.get(key) else {
                return false;
            };
            match property["type"].as_str() {
                Some("integer") => value.as_u64().is_some(),
                Some("string") => value.is_string(),
                Some("boolean") => value.is_boolean(),
                Some("array") => value.is_array(),
                _ => true, // Nested schemas remain the upstream contract.
            }
        })
}

#[test]
fn deployed_schema_refuses_routing_on_create_and_full_update() {
    for name in ["ChannelCreateRequest", "ChannelUpdateRequest"] {
        let mut body = json!({"schema_version":1,"name":"synthetic","profile":{},
            "provider":{},"sources":[],"partition":{},"reply":{}});
        if name == "ChannelCreateRequest" {
            body["credentials"] = json!({});
        } else {
            body["expected_version"] = json!(1);
        }
        assert!(request_matches_schema(name, &body));
        body["routing"] = json!({});
        assert!(!request_matches_schema(name, &body));
        assert!(request_matches_schema(
            name,
            &crate::services::nyxbot_gateway_policy::channel_request(&body)
        ));
    }
    assert!(request_matches_schema(
        "RoutingUpdateRequest",
        &json!({"expected_version":1,"routing":{}})
    ));
    assert!(!request_matches_schema(
        "RoutingUpdateRequest",
        &json!({"routing":{}})
    ));
    assert!(!request_matches_schema(
        "RoutingUpdateRequest",
        &json!({"expected_version":1,"routing":{},"credentials":{}})
    ));
}

struct Fixture {
    state: AppState,
    row: NyxbotChannel,
    calls: GatewayCalls,
    behavior: Arc<RoutingBehavior>,
    gateway: tokio::task::JoinHandle<()>,
    upstream: tokio::task::JoinHandle<()>,
    _discord: wiremock::MockServer,
    _identity: native_tests::NativeDiscordFixtureGuard,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.gateway.abort();
        self.upstream.abort();
    }
}

async fn fixture() -> Fixture {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, path},
    };
    let (mut state, _, upstream) = setup("native_routing_contract").await;
    let (row, _) = native_tests::native_channel(&state).await;
    state.db.collection::<NyxbotChannel>(CHANNELS).update_one(doc! {"_id":&row.id},
        doc! {"$set":{"transport":"direct"},"$unset":{"gateway_source":"","gateway_native_phase":"","binding_id":"","gateway_channel_id":"","gateway_version":""}}).await.unwrap();
    let row = load_channel(&state, OWNER, &row.id).await.unwrap();
    let behavior = Arc::new(RoutingBehavior::default());
    let (calls, gateway) = mock_gateway_with_routing(
        &mut state,
        Arc::new(AtomicBool::new(false)),
        behavior.clone(),
    )
    .await;
    native_tests::native_flag(&state, true).await;
    crate::services::security_readiness_service::ready_test_replica(&state.db).await;
    let discord = MockServer::start().await;
    crate::services::nyxbot_gateway_source::TEST_DISCORD_BASES
        .lock()
        .unwrap()
        .insert(row.channel_bot_id.clone(), discord.uri());
    let identity = native_tests::NativeDiscordFixtureGuard(row.channel_bot_id.clone());
    Mock::given(path("/users/@me"))
        .and(header("authorization", "Bot test-discord-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"100","bot":true})))
        .mount(&discord)
        .await;
    Fixture {
        state,
        row,
        calls,
        behavior,
        gateway,
        upstream,
        _discord: discord,
        _identity: identity,
    }
}

#[tokio::test]
async fn native_routing_lost_response_confirms_exact_revision_before_handoff() {
    let f = fixture().await;
    f.behavior.lose_response.store(true, Ordering::SeqCst);
    f.behavior.pause.store(true, Ordering::SeqCst);
    let (state, row) = (f.state.clone(), f.row.clone());
    let pending = tokio::spawn(async move { move_to_gateway(&state, &row).await });
    tokio::time::timeout(Duration::from_secs(10), f.behavior.arrived.notified())
        .await
        .unwrap();
    let preparing = load_channel(&f.state, OWNER, &f.row.id).await.unwrap();
    assert_eq!(preparing.transport, "direct");
    assert_eq!(preparing.gateway_native_phase.as_deref(), Some("preparing"));
    assert!(!crate::services::nyxbot_gateway_source::ready(&preparing));
    assert_eq!(preparing.route_api_key_id, f.row.route_api_key_id);
    assert_eq!(preparing.owner_sender_ids, f.row.owner_sender_ids);
    f.behavior.pause.store(false, Ordering::SeqCst);
    f.behavior.resume.notify_one();
    pending.await.unwrap().unwrap();
    let ready = load_channel(&f.state, OWNER, &f.row.id).await.unwrap();
    assert!(crate::services::nyxbot_gateway_source::ready(&ready));
    let writes = f.calls.lock().await;
    let routing: Vec<_> = writes
        .iter()
        .filter(|c| c.1.ends_with("/routing"))
        .collect();
    assert_eq!(
        routing[0].2, routing[1].2,
        "retry retains original version and policy"
    );
    assert_eq!(routing[0].2["expected_version"], 1);
    assert_eq!(writes[0].0, "POST");
    assert!(writes[0].2.get("routing").is_none());
    assert_eq!(
        Uuid::parse_str(writes[0].2["record_id"].as_str().unwrap())
            .unwrap()
            .get_version_num(),
        4
    );
    let full: Vec<_> = writes
        .iter()
        .filter(|c| c.0 == "PUT" && !c.1.ends_with("/routing"))
        .collect();
    assert_eq!(full[0].2["expected_version"], 2);
    assert!(full.iter().all(|c| c.2.get("routing").is_none()));
    // Move performs one settings reconciliation after adopting version 3.
    assert_eq!(ready.gateway_version, Some(5));
    assert_eq!(routing.last().unwrap().2["expected_version"], 4);
}

#[tokio::test]
async fn native_routing_refusal_or_bad_ack_preserves_working_direct_route() {
    for mode in ["conflict", "unacknowledged", "wrong_version"] {
        let conflict = mode == "conflict";
        let f = fixture().await;
        f.behavior.conflict.store(conflict, Ordering::SeqCst);
        f.behavior
            .unacknowledged
            .store(mode == "unacknowledged", Ordering::SeqCst);
        f.behavior
            .wrong_version
            .store(mode == "wrong_version", Ordering::SeqCst);
        assert_eq!(
            move_to_gateway(&f.state, &f.row).await,
            Err(if conflict {
                "gateway_conflict"
            } else {
                "gateway_invalid_response"
            })
        );
        let saved = load_channel(&f.state, OWNER, &f.row.id).await.unwrap();
        assert_eq!(saved.transport, "direct");
        assert_eq!(saved.status, "active");
        assert_eq!(saved.route_id, f.row.route_id);
        assert_eq!(saved.route_api_key_id, f.row.route_api_key_id);
        assert_eq!(saved.owner_sender_ids, f.row.owner_sender_ids);
        assert!(!crate::services::nyxbot_gateway_source::ready(&saved));
        assert_eq!(
            saved.gateway_native_activation_operation.as_deref(),
            Some("routing_update")
        );
        assert_eq!(
            saved.gateway_native_activation_detail.as_deref(),
            Some(if conflict {
                "version_conflict"
            } else {
                "policy_not_acknowledged"
            })
        );
        let key = key_service::get_api_key(&f.state.db, OWNER, &f.row.route_api_key_id)
            .await
            .unwrap();
        assert!(key.is_active);
        let calls = f.calls.lock().await;
        assert_eq!(
            calls.iter().filter(|c| c.1.ends_with("/routing")).count(),
            1
        );
        assert!(
            calls.iter().any(|c| c.0 == "GET"),
            "retirement reconciles remote version"
        );
        assert_eq!(
            calls.iter().any(|c| c.0 == "DELETE"),
            conflict,
            "uncertain changed revisions retain the original release fence for review"
        );
        assert!(
            !calls
                .iter()
                .any(|c| c.0 == "PUT" && !c.1.ends_with("/routing"))
        );
    }
}

#[tokio::test]
async fn native_routing_conflict_reconciles_without_overwriting_a_later_policy() {
    let f = fixture().await;
    move_to_gateway(&f.state, &f.row).await.unwrap();
    let ready = load_channel(&f.state, OWNER, &f.row.id).await.unwrap();
    let base = gateway_base(&f.state).await.unwrap();
    let creator = creator_bearer(&f.state, OWNER).unwrap();
    let channel = ready.gateway_channel_id.as_deref().unwrap();
    let bot = channel_bot_service::get_bot(&f.state.db, &ready.channel_bot_id)
        .await
        .unwrap();
    let desired = gateway_policy(
        &f.state,
        &ready,
        &bot,
        &[ready.route_id.clone().unwrap()],
        GATEWAY_GROUPS_DEFAULT,
    );
    let expected = ready.gateway_version.unwrap();
    let put = |version, routing: Value| {
        let (base, creator) = (base.clone(), creator.clone());
        async move {
            crate::services::nyxbot_gateway_policy::put_routing_at_base(
                &base, &creator, channel, version, &routing,
            )
            .await
            .unwrap()
        }
    };
    assert_eq!(put(expected, desired["routing"].clone()).await.status, 200);
    assert_eq!(put(expected, desired["routing"].clone()).await.status, 200);
    let mut other = desired["routing"].clone();
    other["addressing"]["command"] = json!(true);
    assert_eq!(put(expected, other.clone()).await.status, 409);
    assert_eq!(put(expected + 1, other).await.status, 200);
    assert_eq!(put(expected, desired["routing"].clone()).await.status, 409);
    // Settings starts with its stale local version: read, retain the fresh CAS
    // version, and keep acknowledgement false. No blind mutation at that version.
    f.calls.lock().await.clear();
    f.state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id":&ready.id},
            doc! {"$set":{"gateway_no_progress":false}},
        )
        .await
        .unwrap();
    assert_eq!(
        chats::sync_gateway_groups(&f.state, &ready, true)
            .await
            .unwrap(),
        Some("gateway_policy_unavailable")
    );
    let reconciled = load_channel(&f.state, OWNER, &ready.id).await.unwrap();
    assert_eq!(reconciled.gateway_version, Some(expected + 2));
    assert!(!reconciled.gateway_no_progress);
    assert!(
        !f.calls
            .lock()
            .await
            .iter()
            .any(|c| c.1.ends_with("/routing"))
    );
    // A later reconciliation authors the current desired policy from the read.
    assert_eq!(
        chats::sync_gateway_groups(&f.state, &reconciled, true)
            .await
            .unwrap(),
        None
    );
    let saved = load_channel(&f.state, OWNER, &ready.id).await.unwrap();
    assert!(saved.gateway_no_progress);
    assert_eq!(saved.gateway_version, Some(expected + 4));
}

#[test]
fn native_routing_acknowledgement_requires_source_and_saved_policy() {
    let desired = json!({"sources":[{"type":"discord","bot_id":"100","intents":37377,"admission":{"type":"scoped"}}],
        "reply":{"ack_after_ms":0,"auto_final":true,"sinks":[{"type":"source"}]},
        "partition":{"version":1,"key":"conversation_and_sender"},
        "routing":{"version":1,"default_consumer":{"name":"default","mentionable":false},
            "default_targets":["default"],"max_targets":1,"reply_label":"never",
            "addressing":{"mentions":false,"command":false,"event_field":false,"unknown_target_notice":false}}});
    let mut body = json!({"version":2,"definition":desired});
    assert!(chats::gateway_policy_acknowledged(&body, &desired));
    body["definition"]["routing"]["consumers"] = json!([]);
    body["definition"]["routing"]["rules"] = json!([]);
    assert!(chats::gateway_policy_acknowledged(&body, &desired));
    body["definition"]["sources"][0]["bot_id"] = json!("101");
    assert!(!chats::gateway_policy_acknowledged(&body, &desired));
    body["definition"] = desired.clone();
    body["definition"]["routing"]["addressing"]["command"] = json!(true);
    assert!(!chats::gateway_policy_acknowledged(&body, &desired));
    body["definition"]
        .as_object_mut()
        .unwrap()
        .remove("routing");
    assert!(!chats::gateway_policy_acknowledged(&body, &desired));
    let mut missing = desired.clone();
    missing.as_object_mut().unwrap().remove("routing");
    assert!(!chats::gateway_policy_acknowledged(&body, &missing));
    assert_eq!(
        crate::services::nyxbot_gateway_policy::next_version(&json!({"version":3}), 1),
        None
    );
    assert_eq!(
        crate::services::nyxbot_gateway_policy::next_version(&json!({"version":2}), 1),
        Some(2)
    );
}

#[tokio::test]
async fn native_initial_setup_does_not_become_ready_after_routing_refusal() {
    let f = fixture().await;
    f.behavior.conflict.store(true, Ordering::SeqCst);
    f.state
        .db
        .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
        .delete_one(doc! {"_id":&f.row.route_id})
        .await
        .unwrap();
    f.state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .delete_one(doc! {"_id":&f.row.id})
        .await
        .unwrap();
    let agent = crate::services::assistant_team_service::ensure_nyxbot(&f.state.db, OWNER)
        .await
        .unwrap();
    let (saved, _) = connect(&f.state, OWNER, None, &f.row.channel_bot_id, &agent)
        .await
        .unwrap();
    assert_eq!(saved.transport, "direct");
    assert!(!crate::services::nyxbot_gateway_source::ready(&saved));
    assert_eq!(
        saved.gateway_native_activation_operation.as_deref(),
        Some("routing_update")
    );
    assert_eq!(
        saved.gateway_native_activation_detail.as_deref(),
        Some("version_conflict")
    );
    assert!(f.calls.lock().await.iter().any(|c| c.0 == "DELETE"));
}

#[tokio::test]
async fn native_settings_routing_conflict_reads_and_acknowledges_saved_policy() {
    let f = fixture().await;
    move_to_gateway(&f.state, &f.row).await.unwrap();
    let ready = load_channel(&f.state, OWNER, &f.row.id).await.unwrap();
    f.state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id":&ready.id},
            doc! {"$set":{"gateway_no_progress":false}},
        )
        .await
        .unwrap();
    f.behavior.conflict.store(true, Ordering::SeqCst);
    f.calls.lock().await.clear();
    assert_eq!(
        chats::sync_gateway_groups(&f.state, &ready, true)
            .await
            .unwrap(),
        None
    );
    let saved = load_channel(&f.state, OWNER, &ready.id).await.unwrap();
    assert_eq!(saved.gateway_version, ready.gateway_version.map(|v| v + 1));
    assert!(saved.gateway_no_progress);
    let calls = f.calls.lock().await;
    assert_eq!(
        calls.iter().filter(|c| c.1.ends_with("/routing")).count(),
        1
    );
    assert!(
        calls
            .iter()
            .any(|c| c.0 == "GET" && c.1 == ready.gateway_channel_id.as_deref().unwrap())
    );
}
