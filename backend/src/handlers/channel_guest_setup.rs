use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};

use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        channel_bot::ChannelBot,
        channel_guest_policy::GuestDataRule,
        channel_guest_setup::{GuestSetup, SetupConfiguration, SkillDeclarationFile},
    },
    mw::auth::{AuthUser, require_first_party_human},
    services::{
        audit_service::AuditActor,
        channel_guest_access::{
            GuestMode, ObservationHint, google_sheets, list_verified_actor_hints,
            list_verified_room_hints, personal_records, policy, select_mode_for_bot,
            select_rollout_for_bot, setup, skills,
        },
    },
};

#[derive(Serialize)]
pub(crate) struct SetupResponse {
    available: bool,
    enabled: bool,
    bot_revision: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    activation_issue: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    settings: Option<Settings>,
}

#[derive(Serialize)]
struct Settings {
    revision: i64,
    policy_revision: i64,
    prepared_draft_revision: Option<i64>,
    draft: Option<SetupConfiguration>,
    applied: Option<SetupConfiguration>,
    effective_rule_count: usize,
    verified_guests: Vec<ObservationHint>,
    verified_rooms: Vec<ObservationHint>,
    sheets_connections: Vec<setup::SheetsConnection>,
    sources: Vec<Source>,
}

#[derive(Serialize)]
struct Source {
    id: &'static str,
    collection_id: &'static str,
    actions: &'static [&'static str],
    fields: &'static [&'static str],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SaveRequest {
    expected_revision: i64,
    configuration: SetupConfiguration,
    #[serde(default)]
    staging: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreviewRequest {
    configuration: SetupConfiguration,
    guest_ids: Option<[String; 2]>,
    #[serde(default)]
    staging: bool,
}

async fn authorize(state: &AppState, auth: &AuthUser, bot_id: &str) -> AppResult<ChannelBot> {
    require_first_party_human(auth)?;
    let (_, bot) =
        super::channel_bots::resolve_bot_owner_for_write(state, &auth.user_id.to_string(), bot_id)
            .await?;
    if !bot.is_active {
        return Err(AppError::ChannelBotNotFound(bot_id.to_owned()));
    }
    Ok(bot)
}

fn require_add_on(mode: GuestMode) -> AppResult<(i64, i64)> {
    match mode {
        GuestMode::Legacy => Err(AppError::NotFound(
            "Guest setup is unavailable for this bot".into(),
        )),
        GuestMode::AddOn {
            activation_epoch,
            bot_revision,
        } => Ok((activation_epoch, bot_revision)),
    }
}

async fn settings(
    state: &AppState,
    bot: &ChannelBot,
    actor: &str,
    active: bool,
    issue: Option<&'static str>,
) -> AppResult<SetupResponse> {
    let (row, policy, verified_guests, verified_rooms, sheets_connections) = tokio::try_join!(
        setup::load(&state.db, &bot.id),
        policy::load_policy(&state.db, &bot.id),
        list_verified_actor_hints(&state.db, &bot.id),
        list_verified_room_hints(&state.db, &bot.id),
        setup::sheets_connections(&state.db, &bot.id, actor),
    )?;
    let (policy_revision, effective_rule_count) = policy
        .as_ref()
        .map_or((0, 0), |policy| (policy.revision, policy.rules.len()));
    let revision = row.as_ref().map_or(0, |row| row.revision);
    let prepared_draft_revision = row
        .as_ref()
        .and_then(|row| row.prepared.as_ref())
        .map(|prepared| prepared.draft_revision);
    let (draft, applied) = row.map_or((None, None), |GuestSetup { draft, applied, .. }| {
        (draft, applied)
    });
    let mut sources = vec![Source {
        id: personal_records::SOURCE_ID,
        collection_id: personal_records::SOURCE_ID,
        actions: &personal_records::ACTIONS,
        fields: &personal_records::FIELDS,
    }];
    if !sheets_connections.is_empty() {
        sources.push(Source {
            id: google_sheets::SOURCE_ID,
            collection_id: google_sheets::COLLECTION_ID,
            actions: &google_sheets::ACTIONS,
            fields: &[],
        });
    }
    Ok(SetupResponse {
        available: active,
        enabled: bot.guest_access_v1,
        bot_revision: bot.guest_access_v1_revision,
        activation_issue: issue,
        settings: Some(Settings {
            revision,
            policy_revision,
            prepared_draft_revision,
            draft,
            applied,
            effective_rule_count,
            verified_guests,
            verified_rooms,
            sheets_connections,
            sources,
        }),
    })
}

pub(crate) async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(bot_id): Path<String>,
) -> AppResult<Json<SetupResponse>> {
    let bot = authorize(&state, &auth, &bot_id).await?;
    status(&state, &bot, &auth.user_id.to_string(), false)
        .await
        .map(Json)
}

async fn status(
    state: &AppState,
    bot: &ChannelBot,
    actor: &str,
    staging: bool,
) -> AppResult<SetupResponse> {
    let mode = select_mode_for_bot(&state.db, bot).await?;
    let active = matches!(mode, GuestMode::AddOn { .. });
    let issue = if !active
        && matches!(
            select_rollout_for_bot(&state.db, bot).await?,
            GuestMode::AddOn { .. }
        ) {
        Some("prepared_revision_stale")
    } else {
        None
    };
    if active || staging {
        return settings(state, bot, actor, active, issue).await;
    }
    Ok(SetupResponse {
        available: false,
        enabled: bot.guest_access_v1,
        bot_revision: bot.guest_access_v1_revision,
        activation_issue: issue,
        settings: None,
    })
}

pub(crate) async fn staging(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(bot_id): Path<String>,
) -> AppResult<Json<SetupResponse>> {
    let bot = authorize(&state, &auth, &bot_id).await?;
    status(&state, &bot, &auth.user_id.to_string(), true)
        .await
        .map(Json)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreferenceRequest {
    enabled: bool,
    expected_bot_revision: i64,
}

pub(crate) async fn preference(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(bot_id): Path<String>,
    Json(request): Json<PreferenceRequest>,
) -> AppResult<Json<SetupResponse>> {
    let bot = authorize(&state, &auth, &bot_id).await?;
    setup::set_preference(
        &state.db,
        &bot,
        AuditActor::from_auth_user(&auth),
        request.expected_bot_revision,
        request.enabled,
    )
    .await?;
    let bot = authorize(&state, &auth, &bot_id).await?;
    status(&state, &bot, &auth.user_id.to_string(), false)
        .await
        .map(Json)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PrepareRequest {
    expected_revision: i64,
    skill_selection: Option<skills::Update>,
}

pub(crate) async fn prepare(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(bot_id): Path<String>,
    Json(request): Json<PrepareRequest>,
) -> AppResult<Json<SetupResponse>> {
    let bot = authorize(&state, &auth, &bot_id).await?;
    let actor = auth.user_id.to_string();
    let row = setup::load(&state.db, &bot_id)
        .await?
        .filter(|row| row.owner_id == bot.user_id && row.revision == request.expected_revision)
        .ok_or_else(|| AppError::Conflict("Guest setup draft changed".into()))?;
    let configuration = row
        .draft
        .as_ref()
        .ok_or_else(|| AppError::ValidationError("Save a guest setup draft first".into()))?;
    let rules = declaration_rules(&state, &actor, &bot_id, configuration).await?;
    let skill_selection = if let Some(input) = request.skill_selection {
        Some(skills::prepare(&state.db, &actor, &bot_id, &input).await?)
    } else {
        None
    };
    setup::prepare(
        &state.db,
        &bot,
        AuditActor::from_auth_user(&auth),
        request.expected_revision,
        rules,
        skill_selection,
    )
    .await?;
    status(&state, &bot, &actor, true).await.map(Json)
}

async fn declaration_rules(
    state: &AppState,
    actor: &str,
    bot_id: &str,
    configuration: &SetupConfiguration,
) -> AppResult<Vec<GuestDataRule>> {
    setup::validate_size(configuration)?;
    let mut rules = Vec::new();
    for reference in &configuration.declarations {
        if reference.version != 1 {
            return Err(AppError::ValidationError(
                "Unsupported skill declaration version".into(),
            ));
        }
        let package_file = super::agent_skills::read_guest_declaration(
            state,
            actor,
            bot_id,
            reference.skill_revision,
            &reference.skill_id,
        )
        .await?;
        let file: SkillDeclarationFile = serde_json::from_value(package_file).map_err(|_| {
            AppError::ValidationError("Invalid structured guest declaration in pinned skill".into())
        })?;
        if file.version != 1 || file.rules.is_empty() {
            return Err(AppError::ValidationError(
                "The pinned skill needs a nonempty version 1 guest declaration".into(),
            ));
        }
        rules.extend(file.rules);
    }
    Ok(rules)
}

async fn write(
    state: &AppState,
    auth: &AuthUser,
    bot_id: &str,
    request: SaveRequest,
    apply: bool,
) -> AppResult<Json<SetupResponse>> {
    let bot = authorize(state, auth, bot_id).await?;
    let active = if request.staging && !apply {
        None
    } else {
        Some(require_add_on(select_mode_for_bot(&state.db, &bot).await?)?)
    };
    let actor = auth.user_id.to_string();
    let skill_rules = if apply {
        declaration_rules(state, &actor, bot_id, &request.configuration).await?
    } else {
        Vec::new()
    };
    setup::save(
        &state.db,
        setup::SetupWrite {
            bot_id,
            owner_id: &bot.user_id,
            active,
            ownership_version: bot.ownership_version,
            expected_revision: request.expected_revision,
            configuration: request.configuration,
            actor: AuditActor::from_auth_user(auth),
            apply,
            skill_rules,
        },
    )
    .await?;
    status(state, &bot, &actor, request.staging).await.map(Json)
}

pub(crate) async fn save_draft(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(bot_id): Path<String>,
    Json(request): Json<SaveRequest>,
) -> AppResult<Json<SetupResponse>> {
    write(&state, &auth, &bot_id, request, false).await
}

pub(crate) async fn apply(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(bot_id): Path<String>,
    Json(request): Json<SaveRequest>,
) -> AppResult<Json<SetupResponse>> {
    write(&state, &auth, &bot_id, request, true).await
}

pub(crate) async fn preview(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(bot_id): Path<String>,
    Json(request): Json<PreviewRequest>,
) -> AppResult<Json<setup::Preview>> {
    let bot = authorize(&state, &auth, &bot_id).await?;
    if !request.staging {
        require_add_on(select_mode_for_bot(&state.db, &bot).await?)?;
    }
    let actor = auth.user_id.to_string();
    let skill_rules = declaration_rules(&state, &actor, &bot_id, &request.configuration).await?;
    Ok(Json(
        setup::preview_configuration(
            &state.db,
            &bot_id,
            &actor,
            &request.configuration,
            request.guest_ids,
            &skill_rules,
        )
        .await?,
    ))
}

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use bson::{Document, doc};
    use serde_json::{Value, json};
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;

    fn token(state: &AppState, owner: &Uuid) -> String {
        crate::crypto::jwt::generate_access_token(
            &state.jwt_keys,
            &state.config,
            owner,
            "openid account:read",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap()
    }

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
                    .uri(path)
                    .method(method)
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

    #[tokio::test]
    async fn channel_guest_setup_http_draft_preview_apply_and_rollback() {
        use crate::{
            models::{
                feature_flag_override::FlagTargetKind,
                user::{User, UserType},
            },
            services::{
                channel_guest_access::{self as guest, FLAG, PilotSelection},
                feature_flag_admin_service as flag_admin, security_readiness_service,
            },
            test_utils::{connect_transaction_test_database, test_app_state, test_user},
        };

        let db = connect_transaction_test_database("guest_setup_http").await;
        let state = test_app_state(db.clone());
        let (_, private) = crate::routes::build_router();
        let router = private.with_state(state.clone());
        let owner = Uuid::new_v4();
        let other = Uuid::new_v4();
        for person in [owner, other] {
            db.collection::<User>("users")
                .insert_one(test_user(&person.to_string(), UserType::Person))
                .await
                .unwrap();
        }
        let owner_token = token(&state, &owner);
        let other_token = token(&state, &other);
        let bot_id = Uuid::new_v4().to_string();
        db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME).insert_one(doc! {
            "_id":&bot_id,"user_id":owner.to_string(),"platform":"telegram","label":"Guest fixture",
            "platform_bot_id":"123","platform_bot_username":"guest_fixture","status":"active","is_active":true,
            "webhook_registered":false,"webhook_secret_hash":"fixture",
            "bot_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:vec![0;8]},
            "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
        }).await.unwrap();

        let ordinary = format!("/api/v1/channel-bots/{bot_id}");
        let root = format!("{ordinary}/guest-setup");
        let (before_status, before) =
            request(&router, &owner_token, "GET", &ordinary, Value::Null).await;
        assert_eq!(before_status, StatusCode::OK, "{before:?}");
        assert_eq!(
            request(&router, &owner_token, "GET", &root, Value::Null).await,
            (
                StatusCode::OK,
                json!({"available":false,"enabled":false,"bot_revision":0})
            )
        );
        assert_eq!(
            request(&router, &other_token, "GET", &root, Value::Null)
                .await
                .0,
            StatusCode::NOT_FOUND
        );
        let configuration = json!({
            "audience":{"kind":"channel_policy"},
            "rules":[{
                "source_id":"personal_records","collection_id":"personal_records",
                "audience":{"kind":"all_guests"},"scope":{"kind":"own_records"},"delivery":"private",
                "operations":{
                    "read":{"read":["title"],"write":[]},
                    "create":{"read":[],"write":["title"]}
                }
            }],
            "declarations":[],
        });
        let first = json!({"expected_revision":0,"configuration":configuration});
        assert_eq!(
            request(
                &router,
                &owner_token,
                "PUT",
                &format!("{root}/draft"),
                first.clone()
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );

        crate::test_utils::init_test_audit_chain_hmac_key();
        let actor =
            AuditActor::from_auth_user(&crate::test_utils::test_auth_user(&owner.to_string()));
        guest::set_pilot(
            &db,
            PilotSelection {
                bot_ids: Some(vec![bot_id.clone()]),
                fleet: false,
            },
            &actor,
        )
        .await
        .unwrap();
        let _heartbeat = security_readiness_service::controlled_ready_test_replica(&db).await;
        flag_admin::mutate(
            &db,
            FLAG,
            flag_admin::Mutation::Override {
                kind: FlagTargetKind::Global,
                target_id: None,
                enabled: Some(true),
            },
            &actor,
        )
        .await
        .unwrap();

        assert_eq!(
            request(&router, &owner_token, "GET", &root, Value::Null).await,
            (
                StatusCode::OK,
                json!({"available":false,"enabled":false,"bot_revision":0})
            ),
        );
        let (status, preference) = request(
            &router,
            &owner_token,
            "PUT",
            &root,
            json!({"enabled":true,"expected_bot_revision":0}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{preference:?}");
        assert_eq!(preference["available"], true);
        assert_eq!(preference["bot_revision"], 1);
        let (status, ready) = request(&router, &owner_token, "GET", &root, Value::Null).await;
        assert_eq!(status, StatusCode::OK, "{ready:?}");
        assert_eq!(ready["settings"]["effective_rule_count"], 0);
        assert_eq!(ready["settings"]["sources"].as_array().unwrap().len(), 1);
        let (status, draft) = request(
            &router,
            &owner_token,
            "PUT",
            &format!("{root}/draft"),
            first.clone(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{draft:?}");
        assert_eq!(draft["settings"]["draft"], configuration);
        assert_eq!(draft["settings"]["policy_revision"], 0);
        assert_eq!(
            request(&router, &owner_token, "GET", &root, Value::Null)
                .await
                .1,
            draft
        );
        assert_eq!(
            request(
                &router,
                &owner_token,
                "POST",
                &format!("{root}/apply"),
                first
            )
            .await
            .0,
            StatusCode::CONFLICT
        );

        let (status, preview) = request(
            &router,
            &owner_token,
            "POST",
            &format!("{root}/preview"),
            json!({"configuration":configuration}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{preview:?}");
        let check = preview["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|check| check["action"] == "read")
            .unwrap();
        assert_eq!(check["guest_a"], true);
        assert_eq!(check["guest_b"], true);
        assert_eq!(check["guest_b_on_a"], false);
        assert_eq!(check["guest_a_in_group"], false);
        let (status, applied) = request(
            &router,
            &owner_token,
            "POST",
            &format!("{root}/apply"),
            json!({"expected_revision":1,"configuration":configuration}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{applied:?}");
        assert_eq!(applied["settings"]["policy_revision"], 1);
        assert_eq!(applied["settings"]["draft"], Value::Null);
        assert_eq!(applied["settings"]["applied"], configuration);

        prove_preview_against_records(&db, &bot_id).await;

        flag_admin::mutate(
            &db,
            FLAG,
            flag_admin::Mutation::Override {
                kind: FlagTargetKind::Global,
                target_id: None,
                enabled: None,
            },
            &actor,
        )
        .await
        .unwrap();
        assert_eq!(
            request(&router, &owner_token, "GET", &root, Value::Null).await,
            (
                StatusCode::OK,
                json!({"available":false,"enabled":true,"bot_revision":1})
            )
        );
        assert_eq!(
            request(&router, &owner_token, "GET", &ordinary, Value::Null).await,
            (StatusCode::OK, before)
        );
        assert!(
            setup::load(&db, &bot_id)
                .await
                .unwrap()
                .unwrap()
                .applied
                .is_some()
        );
        let staged_config = json!({"audience":{"kind":"owner_only"},"rules":[],"declarations":[]});
        let (status, staged) = request(
            &router,
            &owner_token,
            "PUT",
            &format!("{root}/draft"),
            json!({"staging":true,"expected_revision":2,"configuration":staged_config}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{staged:?}");
        assert_eq!(staged["available"], false);
        assert_eq!(staged["settings"]["policy_revision"], 1);
        let (status, prepared) = request(
            &router,
            &owner_token,
            "POST",
            &format!("{root}/prepare"),
            json!({"expected_revision":3}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{prepared:?}");
        assert_eq!(prepared["available"], false);
        assert_eq!(prepared["settings"]["prepared_draft_revision"], 3);
        assert_eq!(
            policy::load_policy(&db, &bot_id)
                .await
                .unwrap()
                .unwrap()
                .revision,
            1
        );
        flag_admin::mutate(
            &db,
            FLAG,
            flag_admin::Mutation::Override {
                kind: FlagTargetKind::Global,
                target_id: None,
                enabled: Some(true),
            },
            &actor,
        )
        .await
        .unwrap();
        let (status, activated) = request(&router, &owner_token, "GET", &root, Value::Null).await;
        assert_eq!(status, StatusCode::OK, "{activated:?}");
        assert_eq!(activated["available"], true);
        assert_eq!(activated["settings"]["applied"], staged_config);
        assert_eq!(
            activated["settings"]["prepared_draft_revision"],
            Value::Null
        );
        assert!(
            !setup::allows_guest(&db, &bot_id, Some("guest-a"))
                .await
                .unwrap()
        );
        let mut transfer = db.client().start_session().await.unwrap();
        transfer.start_transaction().await.unwrap();
        setup::retire_for_transfer_in_session(&db, &mut transfer, &bot_id)
            .await
            .unwrap();
        transfer.commit_transaction().await.unwrap();
        assert!(setup::load(&db, &bot_id).await.unwrap().is_none());
        let tombstone = policy::load_policy(&db, &bot_id).await.unwrap().unwrap();
        assert_eq!(tombstone.revision, 3);
        assert!(tombstone.rules.is_empty());
    }

    async fn prove_preview_against_records(db: &mongodb::Database, bot_id: &str) {
        use crate::services::channel_guest_access::{
            AnswerAudience, GuestActor, GuestAssurance, GuestTurn, personal_records,
            policy::{self, GuestDataRequest},
        };
        let policy = policy::load_policy(db, bot_id).await.unwrap().unwrap();
        let mut turn = GuestTurn {
            bot_id: bot_id.into(),
            binding_id: "fixture".into(),
            platform: "telegram".into(),
            conversation_id: "private-fixture".into(),
            source_event_id: "event-fixture".into(),
            actor: GuestActor {
                id: "guest-a".into(),
                assurance: GuestAssurance::ProviderVerified,
            },
            audience: AnswerAudience::Private,
            policy_revision: policy.revision,
            activation_epoch: 1,
            bot_revision: 1,
            meta_grant_id: None,
        };
        let mut tx = db.client().start_session().await.unwrap();
        tx.start_transaction().await.unwrap();
        let create = GuestDataRequest {
            source_id: personal_records::SOURCE_ID,
            collection_id: personal_records::SOURCE_ID,
            action: "create",
            record_id: None,
            record_owner_id: None,
            fields: &["title"],
        };
        let permit = policy::authorize(&policy, &turn, &create).unwrap();
        let created = personal_records::execute(
            db,
            &mut tx,
            &permit,
            None,
            personal_records::parse(
                "nyxid__personal_records_create",
                &json!({"title":"Guest A record"}),
            )
            .unwrap(),
        )
        .await
        .unwrap();
        let record = created["record_id"].as_str().unwrap();
        let read = GuestDataRequest {
            action: "read",
            record_id: Some(record),
            fields: &[],
            ..create
        };
        let query = || {
            personal_records::parse("nyxid__personal_records_read", &json!({"record_id":record}))
                .unwrap()
        };
        let permit = policy::authorize(&policy, &turn, &read).unwrap();
        let own = personal_records::execute(db, &mut tx, &permit, None, query())
            .await
            .unwrap();
        assert_eq!(own, json!({"record_id":record,"title":"Guest A record"}));
        turn.audience = AnswerAudience::Shared;
        assert!(policy::authorize(&policy, &turn, &read).is_err());
        turn.audience = AnswerAudience::Private;
        turn.actor.id = "guest-b".into();
        let permit = policy::authorize(&policy, &turn, &read).unwrap();
        assert!(matches!(
            personal_records::execute(db, &mut tx, &permit, None, query()).await,
            Err(AppError::NotFound(_))
        ));
        tx.abort_transaction().await.unwrap();
    }

    #[test]
    fn channel_guest_setup_declaration_ref_rejects_model_authored_rules() {
        let error = serde_json::from_value::<SaveRequest>(json!({
            "expected_revision":0,
            "configuration":{
                "audience":{"kind":"channel_policy"},
                "rules":[],
                "declarations":[{
                    "version":1,"skill_id":"pin","skill_revision":2,"rules":[]
                }]
            }
        }));
        assert!(error.is_err());
    }
}
