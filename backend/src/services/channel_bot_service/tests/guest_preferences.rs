use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::State,
    http::{Request, StatusCode},
    response::IntoResponse,
    routing::post,
};
use bson::{Document, doc};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

use crate::{
    AppState,
    handlers::channel_bots::{self, CreateChannelBotRequest},
    models::{
        channel_bot::COLLECTION_NAME as BOTS,
        feature_flag_override::FlagTargetKind,
        user::{User, UserType},
    },
    mw::auth::AuthUser,
    services::{
        audit_service::AuditActor,
        channel_adapters::discord::DiscordAdapter,
        channel_guest_access::{
            self as guest, AnswerAudience, GuestAdmission, GuestMode, PilotSelection,
            VerifiedGuestInput,
        },
        feature_flag_admin_service as flags, security_readiness_service,
        telegram_new_api::TelegramApi,
    },
    telemetry::TelemetryContext,
    test_utils::{connect_transaction_test_database, test_app_state, test_auth_user, test_user},
};

async fn request(
    router: &Router,
    token: &str,
    method: &str,
    path: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(if body.is_null() {
                    Body::empty()
                } else {
                    Body::from(body.to_string())
                })
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 256 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn set_flag(db: &mongodb::Database, actor: &AuditActor, enabled: bool) {
    flags::mutate(
        db,
        guest::FLAG,
        flags::Mutation::Override {
            kind: FlagTargetKind::Global,
            target_id: None,
            enabled: Some(enabled),
        },
        actor,
    )
    .await
    .unwrap();
}

async fn prepare_turn(db: &mongodb::Database, bot_id: &str, event: &str) -> guest::GuestTurn {
    let mode = guest::select_mode(db, bot_id).await.unwrap();
    let GuestAdmission::AddOn(turn) = guest::classify(
        db,
        mode,
        VerifiedGuestInput {
            bot_id,
            binding_id: "fixture-binding",
            platform: "discord",
            conversation_id: "fixture-chat",
            source_event_id: event,
            sender_id: Some("fixture-guest"),
            audience: AnswerAudience::Private,
            shared_room_verified: false,
            meta_grant_id: None,
        },
    )
    .await
    .unwrap() else {
        panic!("expected add-on admission");
    };
    *turn
}

#[tokio::test]
async fn ac13_same_owner_preference_defaults_rollout_and_independent_fences() {
    let db = connect_transaction_test_database("guest_preference_ac13").await;
    security_readiness_service::ready_test_replica(&db).await;
    crate::test_utils::init_test_audit_chain_hmac_key();
    let state = test_app_state(db.clone());
    let owner = Uuid::new_v4();
    let owner_id = owner.to_string();
    db.collection::<User>(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&owner_id, UserType::Person))
        .await
        .unwrap();
    let actor = AuditActor::from_auth_user(&test_auth_user(&owner_id));
    let token = crate::crypto::jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &owner,
        "openid account:read",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let (_, private) = crate::routes::build_router_with_state(state.clone());
    let settings_routes = private.with_state(state.clone());
    let old_id = Uuid::new_v4().to_string();
    let old_path = format!("/api/v1/channel-bots/{old_id}/guest-setup");
    let now = bson::DateTime::now();
    db.collection::<Document>(BOTS).insert_one(doc!{
        "_id":&old_id,"user_id":&owner_id,"platform":"discord","label":"Legacy bot",
        "platform_bot_id":"11","platform_bot_username":"legacy","is_active":true,"status":"active",
        "webhook_registered":false,"webhook_secret_hash":"legacy",
        "bot_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:vec![0;8]},
        "created_at":now,"updated_at":now,
    }).await.unwrap();

    // Route through the normal POST registration handler with a local Discord provider.
    let discord = MockServer::start().await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/users/@me"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"id":"22","username":"newbot"})),
        )
        .mount(&discord)
        .await;
    let discord_url = discord.uri();
    let registry = Router::new()
        .route(
            "/api/v1/channel-bots",
            post(
                move |State(state): State<AppState>,
                      auth: AuthUser,
                      telemetry: TelemetryContext,
                      Json(body): Json<CreateChannelBotRequest>| {
                    let discord_url = discord_url.clone();
                    async move {
                        let adapter = DiscordAdapter::with_fixture_url(discord_url);
                        let telegram = TelegramApi::new(&state.http_client);
                        channel_bots::create_bot_with_adapter(
                            &state, auth, telemetry, body, &adapter, &telegram,
                        )
                        .await
                        .map(IntoResponse::into_response)
                    }
                },
            ),
        )
        .with_state(state.clone());
    let (status, created) = request(
        &registry,
        &token,
        "POST",
        "/api/v1/channel-bots",
        json!({"platform":"discord","bot_token":"fixture-discord-token","label":"New bot",
            "public_key":"f".repeat(64)}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let new_id = created["id"].as_str().unwrap();
    let new_path = format!("/api/v1/channel-bots/{new_id}/guest-setup");
    let (_, old) = request(&settings_routes, &token, "GET", &old_path, Value::Null).await;
    let (_, new) = request(&settings_routes, &token, "GET", &new_path, Value::Null).await;
    assert_eq!(
        (old["enabled"].as_bool(), old["bot_revision"].as_i64()),
        (Some(false), Some(0))
    );
    assert_eq!(
        (new["enabled"].as_bool(), new["bot_revision"].as_i64()),
        (Some(true), Some(1))
    );
    assert_eq!(
        guest::select_mode(&db, new_id).await.unwrap(),
        GuestMode::Legacy
    );

    guest::set_pilot(
        &db,
        PilotSelection {
            bot_ids: Some(vec![old_id.clone()]),
            fleet: false,
        },
        &actor,
    )
    .await
    .unwrap();
    set_flag(&db, &actor, true).await;
    for id in [&old_id[..], new_id] {
        assert_eq!(
            guest::select_mode(&db, id).await.unwrap(),
            GuestMode::Legacy
        );
    }
    let (status, first) = request(
        &settings_routes,
        &token,
        "PUT",
        &old_path,
        json!({"enabled":true,"expected_bot_revision":0}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["bot_revision"], 1);
    assert!(matches!(
        guest::select_mode(&db, &old_id).await.unwrap(),
        GuestMode::AddOn {
            bot_revision: 1,
            ..
        }
    ));

    set_flag(&db, &actor, false).await;
    guest::set_pilot(
        &db,
        PilotSelection {
            bot_ids: None,
            fleet: true,
        },
        &actor,
    )
    .await
    .unwrap();
    set_flag(&db, &actor, true).await;
    let new_event = prepare_turn(&db, new_id, "event-new").await;
    let old_event = prepare_turn(&db, &old_id, "event-old").await;
    for (enabled, expected) in [(false, 1), (true, 2), (true, 3)] {
        let (status, settings) = request(
            &settings_routes,
            &token,
            "PUT",
            &new_path,
            json!({"enabled":enabled,"expected_bot_revision":expected}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{settings}");
        assert_eq!(
            settings["bot_revision"],
            if enabled && expected == 3 {
                3
            } else {
                expected + 1
            }
        );
        guest::revalidate(&db, &old_event).await.unwrap();
        assert!(guest::revalidate(&db, &new_event).await.is_err());
    }
    let (stale, _) = request(
        &settings_routes,
        &token,
        "PUT",
        &new_path,
        json!({"enabled":false,"expected_bot_revision":1}),
    )
    .await;
    assert_eq!(stale, StatusCode::CONFLICT);

    let new_event = prepare_turn(&db, new_id, "event-new-after-enable").await;
    set_flag(&db, &actor, false).await;
    for turn in [&old_event, &new_event] {
        assert!(guest::revalidate(&db, turn).await.is_err());
        assert_eq!(
            guest::select_mode(&db, &turn.bot_id).await.unwrap(),
            GuestMode::Legacy
        );
    }
    db.drop().await.unwrap();
}
