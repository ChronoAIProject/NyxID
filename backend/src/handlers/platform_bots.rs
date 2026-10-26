use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, header},
};
use serde::{Deserialize, Serialize};

use crate::services::{audit_service, platform_bot_config_service as configs};
use crate::{
    AppState, errors::AppResult, models::platform_bot_config::PlatformBotConfig, mw::auth::AuthUser,
};

pub(crate) async fn admin(state: &AppState, auth: &AuthUser) -> AppResult<()> {
    super::login_client_context::require_first_party_human(auth)?;
    if state.db.collection::<mongodb::bson::Document>(crate::models::user::COLLECTION_NAME)
        .find_one(mongodb::bson::doc! {"_id":auth.user_id.to_string(),"is_active":true,"$or":[{"user_type":"person"},{"user_type":{"$exists":false}}]})
        .projection(mongodb::bson::doc! {"_id":1}).await?.is_none() {
        return Err(crate::errors::AppError::Forbidden("Active human administrator required".into()));
    }
    super::admin_helpers::require_admin(state, auth).await
}

fn private_headers() -> HeaderMap {
    HeaderMap::from_iter([(header::CACHE_CONTROL, "no-store".parse().unwrap())])
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConvertRequest {
    pub channel_bot_id: uuid::Uuid,
    pub funding_owner_user_id: Option<uuid::Uuid>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationRequest {
    pub generation: i64,
}

#[derive(Serialize)]
pub struct ConfigResponse {
    pub display_name: String,
    pub id: String,
    pub channel_bot_id: String,
    pub manager_bot_id: String,
    pub execution_profile: &'static str,
    pub platform: &'static str,
    pub supported_transports: Vec<&'static str>,
    pub transport: &'static str,
    pub generation: i64,
    pub enabled: bool,
    pub funding_owner_user_id: String,
    pub deleted_at: Option<String>,
    pub guide: Option<GuideInput>,
}

impl From<PlatformBotConfig> for ConfigResponse {
    fn from(row: PlatformBotConfig) -> Self {
        let (platform, supported_transports, transport) =
            row.native_entry
                .as_ref()
                .map_or(("telegram", vec!["direct"], "direct"), |entry| {
                    (
                        crate::services::platform_entry_native::platform_name(
                            entry.identity.platform,
                        ),
                        crate::services::platform_entry_gateway_service::supported_transports(
                            entry.identity.platform,
                        )
                        .to_vec(),
                        if matches!(
                            entry.transport,
                            crate::models::platform_entry_identity::EntryTransport::Direct
                        ) {
                            "direct"
                        } else {
                            "gateway"
                        },
                    )
                });
        let execution_profile = if row.native_entry.is_some() {
            "native_entry"
        } else {
            "platform_onboarding"
        };
        Self {
            display_name: format!("{} bot {}", platform, row.manager_bot_id),
            platform,
            supported_transports,
            transport,
            guide: row.guide.map(Into::into),
            id: row.id,
            channel_bot_id: row.channel_bot_id,
            manager_bot_id: row.manager_bot_id,
            execution_profile,
            generation: row.generation,
            enabled: row.enabled,
            funding_owner_user_id: row.funding_owner_user_id,
            deleted_at: row.deleted_at.map(|t| t.to_rfc3339()),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    pub before_id: Option<uuid::Uuid>,
}
#[derive(Serialize)]
pub struct ConfigListResponse {
    pub profiles: Vec<ConfigResponse>,
    pub next_before_id: Option<String>,
}
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<ListQuery>,
) -> AppResult<(HeaderMap, Json<ConfigListResponse>)> {
    admin(&state, &auth).await?;
    let cursor = query.before_id.map(|id| id.to_string());
    let mut rows = configs::list(&state.db, cursor.as_deref()).await?;
    let more = rows.len() > 50;
    rows.truncate(50);
    let next_before_id = if more {
        rows.last().map(|row| row.id.clone())
    } else {
        None
    };
    let labels = configs::display_names(&state.db, &rows).await?;
    Ok((
        private_headers(),
        Json(ConfigListResponse {
            profiles: rows
                .into_iter()
                .map(|row| {
                    let label = labels.get(&row.channel_bot_id).cloned();
                    let mut response: ConfigResponse = row.into();
                    if let Some(label) = label {
                        response.display_name = label;
                    }
                    response
                })
                .collect(),
            next_before_id,
        }),
    ))
}

async fn config_response(
    db: &mongodb::Database,
    row: PlatformBotConfig,
) -> AppResult<ConfigResponse> {
    let label = configs::display_names(db, std::slice::from_ref(&row))
        .await?
        .remove(&row.channel_bot_id);
    let mut response: ConfigResponse = row.into();
    if let Some(label) = label {
        response.display_name = label;
    }
    Ok(response)
}

pub async fn convert(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ConvertRequest>,
) -> AppResult<(HeaderMap, Json<ConfigResponse>)> {
    admin(&state, &auth).await?;
    let row = configs::convert(
        &state.db,
        &auth.user_id.to_string(),
        &body.channel_bot_id.to_string(),
        &body
            .funding_owner_user_id
            .unwrap_or(auth.user_id)
            .to_string(),
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "platform_bot_converted",
        Some(serde_json::json!({"platform_config_id": row.id, "generation": row.generation})),
    );
    Ok((
        private_headers(),
        Json(config_response(&state.db, row).await?),
    ))
}

pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<uuid::Uuid>,
) -> AppResult<(HeaderMap, Json<ConfigResponse>)> {
    admin(&state, &auth).await?;
    Ok((
        private_headers(),
        Json(config_response(&state.db, configs::get(&state.db, &id.to_string()).await?).await?),
    ))
}

async fn disable(
    state: &AppState,
    auth: &AuthUser,
    id: uuid::Uuid,
    generation: i64,
    delete: bool,
) -> AppResult<(HeaderMap, Json<ConfigResponse>)> {
    admin(state, auth).await?;
    let row = configs::disable(
        &state.db,
        &id.to_string(),
        generation,
        delete,
        &auth.user_id.to_string(),
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        auth,
        if delete {
            "platform_bot_deleted"
        } else {
            "platform_bot_paused"
        },
        Some(serde_json::json!({"platform_config_id": row.id, "generation": row.generation})),
    );
    Ok((
        private_headers(),
        Json(config_response(&state.db, row).await?),
    ))
}

pub async fn pause(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<uuid::Uuid>,
    Json(body): Json<GenerationRequest>,
) -> AppResult<(HeaderMap, Json<ConfigResponse>)> {
    disable(&state, &auth, id, body.generation, false).await
}

pub async fn delete(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<uuid::Uuid>,
    Json(body): Json<GenerationRequest>,
) -> AppResult<(HeaderMap, Json<ConfigResponse>)> {
    disable(&state, &auth, id, body.generation, true).await
}

pub async fn verify(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<uuid::Uuid>,
    Json(body): Json<GenerationRequest>,
) -> AppResult<(HeaderMap, Json<ConfigResponse>)> {
    admin(&state, &auth).await?;
    let row = configs::verify_manager(
        &super::telegram_new::service(&state),
        &id.to_string(),
        body.generation,
        &auth.user_id.to_string(),
    )
    .await?;
    Ok((
        private_headers(),
        Json(config_response(&state.db, row).await?),
    ))
}

pub async fn repair(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<uuid::Uuid>,
    Json(body): Json<GenerationRequest>,
) -> AppResult<(HeaderMap, Json<ConfigResponse>)> {
    #[cfg(test)]
    if let Ok(base) = tests::REPAIR_API_BASE.try_with(Clone::clone) {
        return repair_with_service(
            &state,
            &auth,
            id,
            body,
            &crate::services::telegram_new_tests::service(&state, &base),
        )
        .await;
    }
    repair_with_service(
        &state,
        &auth,
        id,
        body,
        &super::telegram_new::service(&state),
    )
    .await
}

async fn repair_with_service(
    state: &AppState,
    auth: &AuthUser,
    id: uuid::Uuid,
    body: GenerationRequest,
    service: &crate::services::telegram_new_service::TelegramNewService<'_>,
) -> AppResult<(HeaderMap, Json<ConfigResponse>)> {
    admin(state, auth).await?;
    let row = configs::get(&state.db, &id.to_string()).await?;
    configs::require_telegram_manager(&row)?;
    if row.generation != body.generation || row.deleted_at.is_some() {
        return Err(crate::errors::AppError::Conflict(
            "Platform configuration changed".into(),
        ));
    }
    let service =
        service
            .for_manager(row.manager_bot_id.parse().map_err(|_| {
                crate::errors::AppError::Conflict("Invalid manager identity".into())
            })?)
            .await?;
    service
        .configure_manager(
            &auth.user_id.to_string(),
            &std::collections::BTreeMap::new(),
            false,
        )
        .await?;
    let row = configs::get(&state.db, &id.to_string()).await?;
    let row = configs::verify_manager(&service, &row.id, row.generation, &auth.user_id.to_string())
        .await?;
    audit_service::log_for_user(
        state.db.clone(),
        auth,
        "platform_bot_repaired",
        Some(serde_json::json!({"platform_config_id": row.id, "generation": row.generation})),
    );
    Ok((
        private_headers(),
        Json(config_response(&state.db, row).await?),
    ))
}

pub(crate) async fn sweep_cleanup(state: &AppState) -> AppResult<()> {
    for _ in 0..8 {
        let Some(task) = configs::claim_cleanup(&state.db).await? else {
            break;
        };
        let outcome = async {
            match task.kind.as_str() {
                "conversation" => {
                    configs::admit_cleanup_effect(&state.db, &task).await?;
                    if let Some(turn) = &task.turn_id {
                        crate::services::assistant_nyxagent::force_stop(&state.db, &task.owner_user_id, &task.resource_id, turn).await?;
                    }
                }
                "machine" => {
                    configs::admit_cleanup_effect(&state.db, &task).await?;
                    let mut nodes: std::collections::BTreeSet<String> = task.machine_node_ids.iter().cloned().collect();
                    nodes.extend(task.machine_jobs.iter().map(|job| job.node_id.clone()));
                    for node_id in nodes {
                        configs::admit_cleanup_effect(&state.db, &task).await?;
                        let node = state.db.collection::<crate::models::node::Node>(crate::models::node::COLLECTION_NAME)
                            .find_one(mongodb::bson::doc! {"_id": &node_id}).await?;
                        if let Some(node) = node {
                            let ids: Vec<_> = task.machine_jobs.iter().filter(|job| job.node_id == node_id).map(|job| job.id.clone()).collect();
                            let result = super::machine_tools::dispatch(state, &node, nyxid_machine::Operation::CancelExact,
                                serde_json::json!({"conversation_id": task.resource_id, "turn_id": task.turn_id.as_deref().unwrap_or(""), "job_ids": ids})).await?;
                            if result["stopped"] != true { return Err(crate::errors::AppError::Conflict("Exact machine cancellation requires recovery".into())); }
                        }
                    }
                }
                "gateway" => { super::nyxbot::cleanup_platform_gateway(state, &task).await?; }
                _ => {
                    return Err(crate::errors::AppError::Conflict(
                        "Unknown platform cleanup operation".into(),
                    ));
                }
            }
            configs::complete_cleanup(&state.db, &task).await
        }
        .await;
        if let Err(error) = outcome {
            tracing::warn!(platform_config_id = %task.platform_config_id, error_code = error.error_code(), "Platform conversion cleanup will retry");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        models::user::{User, UserType},
        mw::auth::AuthMethod,
        test_utils::{
            connect_transaction_test_database, test_app_state, test_auth_user, test_user,
        },
    };
    use mongodb::bson::doc;
    tokio::task_local! { pub(super) static REPAIR_API_BASE: String; }

    #[tokio::test]
    async fn admin_targets_require_live_admin_and_page_eligible_transports_without_losing_tombstones()
     {
        let state =
            test_app_state(connect_transaction_test_database("platform_admin_targets").await);
        crate::services::role_service::seed_system_roles(&state.db)
            .await
            .unwrap();
        let actor = uuid::Uuid::new_v4().to_string();
        let mut user = test_user(&actor, UserType::Person);
        state
            .db
            .collection::<User>(crate::models::user::COLLECTION_NAME)
            .insert_one(&user)
            .await
            .unwrap();
        assert!(
            conversion_targets(
                State(state.clone()),
                test_auth_user(&actor),
                Query(ListQuery { before_id: None })
            )
            .await
            .is_err()
        );
        user.role_ids = vec![
            crate::services::role_service::get_platform_role_ids(&state.db)
                .await
                .unwrap()
                .admin,
        ];
        state
            .db
            .collection::<User>(crate::models::user::COLLECTION_NAME)
            .replace_one(doc! {"_id":&actor}, &user)
            .await
            .unwrap();
        configs::ensure_indexes(&state.db).await.unwrap();
        let bots = state
            .db
            .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME);
        let mut expected = std::collections::BTreeSet::new();
        for number in 0..56 {
            let mut bot = crate::services::platform_entry_native::tests::bot();
            bot.platform_bot_id = format!("remote-{number}");
            if number == 0 {
                bot.platform = "telegram".into();
                bot.credential_source = "telegram_manager".into();
            } else if number == 1 {
                bot.platform = "x".into();
                bot.webhook_registered = true;
            }
            expected.insert(bot.id.clone());
            bots.insert_one(bson::to_document(&bot).unwrap())
                .await
                .unwrap();
        }
        for case in [
            "converted",
            "inactive",
            "unsupported",
            "child",
            "reserved",
            "pending",
            "polling-only-x",
        ] {
            let mut bot = crate::services::platform_entry_native::tests::bot();
            bot.platform_bot_id = case.into();
            let mut row = bson::to_document(&bot).unwrap();
            match case {
                "converted" => {
                    row.insert("platform_profile_id", "existing");
                }
                "inactive" => {
                    row.insert("is_active", false);
                }
                "unsupported" => {
                    row.insert("platform", "aurinko");
                }
                "child" => {
                    row.insert("platform", "telegram-new");
                }
                "polling-only-x" => {
                    row.insert("platform", "x");
                    row.insert("webhook_registered", false);
                }
                "reserved" => {
                    state.db.collection::<bson::Document>(crate::models::platform_bot_config::COLLECTION_NAME).insert_one(doc!{"_id":"retired-profile","channel_bot_id":"old-channel","manager_bot_id":case,"native_entry":{"identity":{"platform":"slack","platform_bot_id":case}},"deleted_at":bson::DateTime::now()}).await.unwrap();
                }
                "pending" => {
                    row.insert("platform_entry_pending", doc! {"actor":&actor});
                    expected.insert(bot.id.clone());
                }
                _ => unreachable!(),
            }
            bots.insert_one(row).await.unwrap();
        }
        let mut found = std::collections::BTreeSet::new();
        let mut before = None;
        let mut pages = 0;
        loop {
            let (_, Json(page)) = conversion_targets(
                State(state.clone()),
                test_auth_user(&actor),
                Query(ListQuery { before_id: before }),
            )
            .await
            .unwrap();
            pages += 1;
            assert!(page.bots.len() <= 50);
            for bot in page.bots {
                found.insert(bot.id);
            }
            before = page
                .next_before_id
                .map(|id| uuid::Uuid::parse_str(&id).unwrap());
            if before.is_none() {
                break;
            }
        }
        assert!(pages >= 2);
        assert_eq!(found, expected);
        state
            .db
            .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
            .update_one(doc! {"_id":&actor}, doc! {"$set":{"role_ids":[]}})
            .await
            .unwrap();
        assert!(
            conversion_targets(
                State(state.clone()),
                test_auth_user(&actor),
                Query(ListQuery { before_id: None })
            )
            .await
            .is_err()
        );
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn native_repair_http_refuses_before_telegram_effects_and_legacy_repair_still_works() {
        use crate::models::platform_bot_config::PlatformExecutionProfile;
        use axum::{
            Router,
            body::Body,
            http::{Request, StatusCode},
            routing::post,
        };
        use tower::ServiceExt;
        use wiremock::{
            Mock, ResponseTemplate,
            matchers::{method, path},
        };
        let (state, actor, server) = crate::services::telegram_new_tests::fixture().await;
        crate::services::role_service::seed_system_roles(&state.db)
            .await
            .unwrap();
        let roles = crate::services::role_service::get_platform_role_ids(&state.db)
            .await
            .unwrap();
        state
            .db
            .collection::<User>(crate::models::user::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&actor},
                doc! {"$set":{"role_ids":[roles.admin]}},
            )
            .await
            .unwrap();
        let bot =
            crate::services::telegram_new_tests::register_manager_channel(&state, &actor, &server)
                .await;
        let funding =
            crate::services::org_service::create_org_user(&state.db, "Repair funding", None, None)
                .await
                .unwrap()
                .id;
        crate::services::org_service::create_membership(
            &state.db,
            &funding,
            &actor,
            crate::models::org_membership::OrgRole::Admin,
            crate::models::org_membership::MemberScopeSource::Inherit,
            None,
        )
        .await
        .unwrap();
        let legacy = configs::convert(&state.db, &actor, &bot.id, &funding)
            .await
            .unwrap();
        let mut native = legacy.clone();
        native.id = uuid::Uuid::new_v4().to_string();
        native.channel_bot_id = uuid::Uuid::new_v4().to_string();
        native.identity_version = Some(2);
        native.execution_profile = PlatformExecutionProfile::NativeEntry;
        native.native_entry = Some(
            crate::models::platform_entry_identity::NativeEntryConfiguration {
                identity: crate::models::platform_entry_identity::NativeEntryIdentity {
                    version: 2,
                    platform: crate::models::platform_entry_identity::EntryPlatform::X,
                    channel_bot_id: native.channel_bot_id.clone(),
                    platform_bot_id: native.manager_bot_id.clone(),
                },
                transport: crate::models::platform_entry_identity::EntryTransport::Direct,
                transport_generation: 1,
                credential_fingerprint: "11".repeat(32),
                control_surface:
                    crate::models::platform_entry_identity::NativeControlSurface::PrivateMessage,
            },
        );
        state
            .db
            .collection::<PlatformBotConfig>(crate::models::platform_bot_config::COLLECTION_NAME)
            .insert_one(&native)
            .await
            .unwrap();
        let before_profile =
            bson::to_document(&configs::get(&state.db, &legacy.id).await.unwrap()).unwrap();
        let before_credential = state
            .db
            .collection::<bson::Document>(crate::models::platform_credential::COLLECTION_NAME)
            .find_one(doc! {"provider":"telegram-new"})
            .await
            .unwrap()
            .unwrap();
        let before = server.received_requests().await.unwrap().len();
        let auth = test_auth_user(&actor);
        let app = Router::new()
            .route(
                "/platform-bots/{id}/repair",
                post(
                    move |State(state): State<AppState>,
                          Path(id): Path<uuid::Uuid>,
                          Json(body): Json<GenerationRequest>| {
                        let auth = auth.clone();
                        async move { repair(State(state), auth, Path(id), Json(body)).await }
                    },
                ),
            )
            .with_state(state.clone());
        let request = |id: &str, generation: i64| {
            Request::builder()
                .method("POST")
                .uri(format!("/platform-bots/{id}/repair"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"generation":generation}).to_string(),
                ))
                .unwrap()
        };
        let refused = REPAIR_API_BASE
            .scope(
                server.uri(),
                tokio::time::timeout(
                    std::time::Duration::from_secs(25),
                    app.clone().oneshot(request(&native.id, native.generation)),
                ),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(refused.status(), StatusCode::CONFLICT);
        let base = server.uri();
        assert!(
            configs::verify_manager(
                &crate::services::telegram_new_tests::service(&state, &base),
                &native.id,
                native.generation,
                &actor
            )
            .await
            .is_err()
        );
        assert_eq!(server.received_requests().await.unwrap().len(), before);
        assert_eq!(
            bson::to_document(&configs::get(&state.db, &legacy.id).await.unwrap()).unwrap(),
            before_profile
        );
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(crate::models::platform_credential::COLLECTION_NAME)
                .find_one(doc! {"provider":"telegram-new"})
                .await
                .unwrap()
                .unwrap(),
            before_credential
        );
        Mock::given(method("POST")).and(path("/bot100:manager-test-secret/getMe")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok":true,"result":{"id":100,"is_bot":true,"username":"NyxSetupBot","can_manage_bots":true}}))).mount(&server).await;
        Mock::given(method("POST"))
            .and(path("/bot100:manager-test-secret/setWebhook"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"ok":true,"result":true})),
            )
            .mount(&server)
            .await;
        let repaired = REPAIR_API_BASE
            .scope(
                server.uri(),
                tokio::time::timeout(
                    std::time::Duration::from_secs(25),
                    app.oneshot(request(&legacy.id, legacy.generation)),
                ),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(repaired.status(), StatusCode::OK);
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .skip(before)
                .any(|request| request.url.path().ends_with("/setWebhook"))
        );
        assert!(
            configs::get(&state.db, &legacy.id)
                .await
                .unwrap()
                .generation
                > legacy.generation
        );
        assert_eq!(
            configs::get(&state.db, &native.id)
                .await
                .unwrap()
                .generation,
            native.generation
        );
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn platform_onboarding_control_reads_require_live_first_party_admin() {
        let db = connect_transaction_test_database("platform_admin_controls").await;
        crate::services::role_service::seed_system_roles(&db)
            .await
            .unwrap();
        let roles = crate::services::role_service::get_platform_role_ids(&db)
            .await
            .unwrap();
        let actor = uuid::Uuid::new_v4().to_string();
        let mut user = test_user(&actor, UserType::Person);
        user.role_ids = vec![roles.operator.clone()];
        db.collection::<User>(crate::models::user::COLLECTION_NAME)
            .insert_one(user)
            .await
            .unwrap();
        let state = test_app_state(db);
        let mut auth = test_auth_user(&actor);
        assert!(admin(&state, &auth).await.is_err());
        assert!(
            get(
                State(state.clone()),
                auth.clone(),
                Path(uuid::Uuid::new_v4())
            )
            .await
            .is_err()
        );
        state
            .db
            .collection::<User>(crate::models::user::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &actor},
                doc! {"$set": {"role_ids": [&roles.admin]}},
            )
            .await
            .unwrap();
        assert!(admin(&state, &auth).await.is_ok());
        for method in [
            AuthMethod::ApiKey,
            AuthMethod::Delegated,
            AuthMethod::Relay,
            AuthMethod::ServiceAccount,
        ] {
            auth.auth_method = method;
            assert!(admin(&state, &auth).await.is_err());
        }
        auth.auth_method = AuthMethod::AccessToken;
        auth.oauth_client_id = Some("external-client".into());
        assert!(admin(&state, &auth).await.is_err());
        auth.oauth_client_id = None;
        state
            .db
            .collection::<User>(crate::models::user::COLLECTION_NAME)
            .update_one(doc! {"_id": &actor}, doc! {"$set": {"role_ids": []}})
            .await
            .unwrap();
        assert!(admin(&state, &auth).await.is_err());
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuideInput {
    pub model_service_id: uuid::Uuid,
    pub model_id: String,
    pub limits: super::platform_setups::LimitsInput,
    pub anonymous_daily_credits: crate::models::credits::Credits,
    pub platform_turns_per_hour: u32,
    pub platform_concurrent_turns: u32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigureGuideInput {
    pub generation: i64,
    pub enabled: bool,
    pub guide: GuideInput,
}
impl From<crate::models::platform_bot_config::GuideConfiguration> for GuideInput {
    fn from(row: crate::models::platform_bot_config::GuideConfiguration) -> Self {
        Self {
            model_service_id: uuid::Uuid::parse_str(&row.model_service_id).unwrap_or_default(),
            model_id: row.model_id,
            limits: row.limits.into(),
            anonymous_daily_credits: row.anonymous_daily_credits,
            platform_turns_per_hour: row.platform_turns_per_hour,
            platform_concurrent_turns: row.platform_concurrent_turns,
        }
    }
}
pub async fn configure_guide(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<uuid::Uuid>,
    Json(body): Json<ConfigureGuideInput>,
) -> AppResult<(HeaderMap, Json<ConfigResponse>)> {
    admin(&state, &auth).await?;
    let guide = crate::models::platform_bot_config::GuideConfiguration {
        model_service_id: body.guide.model_service_id.to_string(),
        model_id: body.guide.model_id,
        limits: body.guide.limits.into(),
        anonymous_daily_credits: body.guide.anonymous_daily_credits,
        platform_turns_per_hour: body.guide.platform_turns_per_hour,
        platform_concurrent_turns: body.guide.platform_concurrent_turns,
    };
    let row = configs::configure_guide(
        &state,
        &auth.user_id.to_string(),
        &id.to_string(),
        body.generation,
        guide,
        body.enabled,
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "platform_guide_configured",
        Some(
            serde_json::json!({"platform_config_id": row.id, "generation": row.generation, "enabled": row.enabled}),
        ),
    );
    Ok((
        private_headers(),
        Json(config_response(&state.db, row).await?),
    ))
}

#[derive(Serialize)]
pub struct GuideModelsResponse {
    pub generation: i64,
    pub models: Vec<String>,
    pub model_service_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuideModelsInput {
    pub generation: i64,
    pub model_service_id: Option<uuid::Uuid>,
}
pub async fn discover_guide_models(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<uuid::Uuid>,
    Json(body): Json<GuideModelsInput>,
) -> AppResult<(HeaderMap, Json<GuideModelsResponse>)> {
    admin(&state, &auth).await?;
    let mut profile = configs::get(&state.db, &id.to_string()).await?;
    if profile.generation != body.generation || profile.deleted_at.is_some() {
        return Err(crate::errors::AppError::Conflict(
            "Reload the current platform configuration".into(),
        ));
    }
    profile.configured_by = auth.user_id.to_string();
    if let Some(service_id) = body.model_service_id {
        let mut guide = profile.guide.clone().unwrap_or(
            crate::models::platform_bot_config::GuideConfiguration {
                model_service_id: service_id.to_string(),
                model_id: "readiness-discovery".into(),
                limits: crate::services::platform_setup_service::default_limits(),
                anonymous_daily_credits: "0.1".parse().unwrap(),
                platform_turns_per_hour: 20,
                platform_concurrent_turns: 1,
            },
        );
        guide.model_service_id = service_id.to_string();
        profile.guide = Some(guide);
    }
    let service_id = profile
        .guide
        .as_ref()
        .ok_or_else(|| {
            crate::errors::AppError::Conflict("Choose a platform inference service".into())
        })?
        .model_service_id
        .clone();
    let models =
        crate::services::platform_inference_service::discover_models(&state, &profile).await?;
    Ok((
        private_headers(),
        Json(GuideModelsResponse {
            generation: profile.generation,
            models,
            model_service_id: service_id,
        }),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogModelsQuery {
    pub after_id: Option<uuid::Uuid>,
}
#[derive(Serialize)]
pub struct CatalogModelService {
    pub id: String,
    pub name: String,
}
#[derive(Serialize)]
pub struct CatalogModelServicesResponse {
    pub services: Vec<CatalogModelService>,
    pub next_after_id: Option<String>,
}
pub async fn eligible_guide_services(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<uuid::Uuid>,
    Query(query): Query<CatalogModelsQuery>,
) -> AppResult<(HeaderMap, Json<CatalogModelServicesResponse>)> {
    admin(&state, &auth).await?;
    let mut profile = configs::get(&state.db, &id.to_string()).await?;
    if profile.deleted_at.is_some() {
        return Err(crate::errors::AppError::Conflict(
            "This manager is retired".into(),
        ));
    }
    profile.configured_by = auth.user_id.to_string();
    let after = query.after_id.map(|id| id.to_string());
    let (rows, next_after_id) =
        crate::services::platform_inference_service::eligible_catalog_models(
            &state,
            &profile,
            after.as_deref(),
        )
        .await?;
    Ok((
        private_headers(),
        Json(CatalogModelServicesResponse {
            services: rows
                .into_iter()
                .map(|(id, name)| CatalogModelService { id, name })
                .collect(),
            next_after_id,
        }),
    ))
}

#[derive(Serialize)]
pub struct CleanupTaskResponse {
    pub id: String,
    pub remote_channel_id: Option<String>,
    pub captured_version: Option<i64>,
}
#[derive(Serialize)]
pub struct CleanupReviewResponse {
    pub task_id: String,
    pub profile_generation: i64,
    pub remote_channel_id: String,
    pub captured_version: i64,
    pub observed_version: Option<i64>,
    pub remote_exists: bool,
    pub completed: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CleanupResolutionInput {
    pub profile_generation: i64,
    pub captured_version: i64,
    pub observed_version: Option<i64>,
    pub action: String,
}
pub async fn cleanup_tasks(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<uuid::Uuid>,
) -> AppResult<(HeaderMap, Json<Vec<CleanupTaskResponse>>)> {
    admin(&state, &auth).await?;
    let rows = crate::services::nyxbot_gateway_service::list_cleanup(
        &state,
        &auth.user_id.to_string(),
        &id.to_string(),
    )
    .await?;
    Ok((
        private_headers(),
        Json(
            rows.into_iter()
                .map(|row| CleanupTaskResponse {
                    id: row.id,
                    remote_channel_id: row.gateway_channel_id,
                    captured_version: row.gateway_version,
                })
                .collect(),
        ),
    ))
}
pub async fn inspect_cleanup(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, task)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> AppResult<(HeaderMap, Json<CleanupReviewResponse>)> {
    admin(&state, &auth).await?;
    let review = crate::services::nyxbot_gateway_service::inspect_cleanup(
        &state,
        &auth.user_id.to_string(),
        &id.to_string(),
        &task.to_string(),
    )
    .await?;
    Ok((
        private_headers(),
        Json(CleanupReviewResponse {
            task_id: review.task_id,
            profile_generation: review.profile_generation,
            remote_channel_id: review.remote_channel_id,
            captured_version: review.captured_version,
            observed_version: review.observed_version,
            remote_exists: review.remote_exists,
            completed: review.completed,
        }),
    ))
}
pub async fn resolve_cleanup(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, task)): Path<(uuid::Uuid, uuid::Uuid)>,
    Json(body): Json<CleanupResolutionInput>,
) -> AppResult<axum::http::StatusCode> {
    admin(&state, &auth).await?;
    crate::services::nyxbot_gateway_service::resolve_cleanup(
        &state,
        &auth.user_id.to_string(),
        &id.to_string(),
        &task.to_string(),
        body.profile_generation,
        body.captured_version,
        body.observed_version,
        &body.action,
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "platform_gateway_cleanup_reviewed",
        Some(serde_json::json!({"profile_id": id, "task_id": task, "action": body.action})),
    );
    Ok(axum::http::StatusCode::NO_CONTENT)
}

// Public/personal v1 routes are mounted only with real producer composition.
use crate::services::{
    platform_bot_runtime_control_service as runtime_controls,
    platform_manager_contract as manager_contract,
};
use axum::Extension;
use std::sync::Arc;

type ManagerProducers = Arc<manager_contract::PlatformManagerProducers>;

fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicSource {
    Nyxagent,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicPolicyInput {
    #[serde(default, deserialize_with = "present")]
    source: Option<PublicSource>,
    #[serde(default, deserialize_with = "present")]
    profile: Option<String>,
    #[serde(default, deserialize_with = "present")]
    sponsor_owner_user_id: Option<uuid::Uuid>,
    #[serde(default, deserialize_with = "present")]
    total_daily_credits: Option<crate::models::credits::Credits>,
    #[serde(default, deserialize_with = "present")]
    sender_daily_credits: Option<crate::models::credits::Credits>,
}
#[derive(Default)]
pub enum PublicPolicyField {
    #[default]
    Unchanged,
    Clear,
    Update(PublicPolicyInput),
}
fn policy_field<'de, D: serde::Deserializer<'de>>(d: D) -> Result<PublicPolicyField, D::Error> {
    Ok(match Option::<PublicPolicyInput>::deserialize(d)? {
        None => PublicPolicyField::Clear,
        Some(p) => PublicPolicyField::Update(p),
    })
}
impl PublicPolicyField {
    fn patch(self) -> manager_contract::PolicyPatch {
        match self {
            Self::Unchanged => manager_contract::PolicyPatch::Unchanged,
            Self::Clear => manager_contract::PolicyPatch::Clear,
            Self::Update(p) => {
                let _ = p.source;
                manager_contract::PolicyPatch::Update(manager_contract::PublicPolicyPatch {
                    sponsor_owner_user_id: p.sponsor_owner_user_id.map(|id| id.to_string()),
                    nyxagent_profile: p.profile,
                    total_daily_credits: p.total_daily_credits,
                    sender_daily_credits: p.sender_daily_credits,
                })
            }
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeInput {
    #[serde(default, deserialize_with = "present")]
    public_deployment_digest: Option<String>,
    #[serde(default)]
    initialize: bool,
    #[serde(default, deserialize_with = "present")]
    expected_profile_generation: Option<i64>,
    expected_manager_authority_epoch: i64,
    expected_personal_control_revision: i64,
    expected_public_policy_revision: i64,
    #[serde(default, deserialize_with = "present")]
    personal_enabled: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    public_enabled: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    emergency_stopped: Option<bool>,
    #[serde(default, deserialize_with = "policy_field")]
    public_policy: PublicPolicyField,
}
impl RuntimeInput {
    fn patch(self) -> manager_contract::RuntimePatch {
        manager_contract::RuntimePatch {
            public_deployment_digest: self.public_deployment_digest,
            initialize: self.initialize,
            expected_profile_generation: self.expected_profile_generation,
            expected: manager_contract::ExpectedRuntimeRevisions {
                manager_authority_epoch: self.expected_manager_authority_epoch,
                personal_control_revision: self.expected_personal_control_revision,
                public_policy_revision: self.expected_public_policy_revision,
            },
            personal_enabled: self.personal_enabled,
            public_enabled: self.public_enabled,
            emergency_stopped: self.emergency_stopped,
            public_policy: self.public_policy.patch(),
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicReviewInput {
    #[serde(default)]
    initialize: bool,
    #[serde(default, deserialize_with = "present")]
    expected_profile_generation: Option<i64>,
    expected_manager_authority_epoch: i64,
    expected_personal_control_revision: i64,
    expected_public_policy_revision: i64,
    #[serde(default, deserialize_with = "policy_field")]
    public_policy: PublicPolicyField,
}
impl PublicReviewInput {
    fn patch(self) -> manager_contract::RuntimePatch {
        manager_contract::RuntimePatch {
            public_deployment_digest: None,
            initialize: self.initialize,
            expected_profile_generation: self.expected_profile_generation,
            expected: manager_contract::ExpectedRuntimeRevisions {
                manager_authority_epoch: self.expected_manager_authority_epoch,
                personal_control_revision: self.expected_personal_control_revision,
                public_policy_revision: self.expected_public_policy_revision,
            },
            personal_enabled: None,
            public_enabled: None,
            emergency_stopped: None,
            public_policy: self.public_policy.patch(),
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClearEmergencyInput {
    expected_manager_authority_epoch: i64,
    expected_personal_control_revision: i64,
    expected_public_policy_revision: i64,
}
#[derive(Serialize)]
pub struct PublicPolicyResponse {
    source: &'static str,
    profile: String,
    sponsor_owner_user_id: String,
    total_daily_credits: crate::models::credits::Credits,
    sender_daily_credits: crate::models::credits::Credits,
}
#[derive(Serialize)]
pub struct RuntimeResponse {
    activated_policy_digest: Option<String>,
    manager_authority_epoch: i64,
    personal_control_revision: i64,
    public_policy_revision: i64,
    personal_enabled: bool,
    public_enabled: bool,
    emergency_stopped: bool,
    remembered_personal_enabled: Option<bool>,
    remembered_public_enabled: Option<bool>,
    public_policy: Option<PublicPolicyResponse>,
}
impl From<crate::models::platform_bot_runtime_controls::PlatformBotRuntimeControls>
    for RuntimeResponse
{
    fn from(r: crate::models::platform_bot_runtime_controls::PlatformBotRuntimeControls) -> Self {
        Self {
            activated_policy_digest: None,
            manager_authority_epoch: r.manager_authority_epoch,
            personal_control_revision: r.personal_control_revision,
            public_policy_revision: r.public_policy_revision,
            personal_enabled: r.personal_enabled,
            public_enabled: r.public_enabled,
            emergency_stopped: r.emergency_stopped,
            remembered_personal_enabled: r.remembered_personal_enabled,
            remembered_public_enabled: r.remembered_public_enabled,
            public_policy: r.public_policy.map(|p| PublicPolicyResponse {
                source: "nyxagent",
                profile: p.nyxagent_profile,
                sponsor_owner_user_id: p.sponsor_owner_user_id,
                total_daily_credits: p.total_daily_credits,
                sender_daily_credits: p.sender_daily_credits,
            }),
        }
    }
}
#[derive(Serialize)]
pub struct PublicReadinessResponse {
    status: &'static str,
    reason: Option<&'static str>,
    checked_at: Option<String>,
    expires_at: Option<String>,
    evidence_digest: Option<String>,
    policy_digest: Option<String>,
}
impl PublicReadinessResponse {
    fn unavailable(reason: manager_contract::UnavailableReason) -> Self {
        use manager_contract::UnavailableReason::*;
        let reason = match reason {
            ConfirmationRejected => "confirmation_rejected",
            ControlInvalid => "control_invalid",
            ControlRateLimited => "control_rate_limited",
            ModeDisabled => "mode_disabled",
            ManagerUnavailable => "manager_unavailable",
            EmergencyStopped => "emergency_stopped",
            PersonalDisabled => "personal_disabled",
            PublicDisabled => "public_disabled",
            OwnerUnavailable => "owner_unavailable",
            TransportUnavailable => "transport_unavailable",
            OriginRevoked => "origin_revoked",
            EngineUnavailable => "engine_unavailable",
            UnsupportedInput => "unsupported_input",
            InsufficientFunds => "insufficient_funds",
            CapacityLimited => "capacity_limited",
            PublicNotReady => "public_not_ready",
            PublicQuotaLimited => "public_quota_limited",
            RuntimeUninitialized => "runtime_uninitialized",
            SponsorUnavailable => "sponsor_unavailable",
            PublicContractUnsupported => "public_contract_unsupported",
            PublicAuthorityRejected => "public_authority_rejected",
            ProfileUnavailable => "profile_unavailable",
            ProofExpired => "proof_expired",
            PublicTurnFailed => "public_turn_failed",
        };
        Self {
            status: "unavailable",
            reason: Some(reason),
            checked_at: None,
            expires_at: None,
            evidence_digest: None,
            policy_digest: None,
        }
    }
    fn from_readiness(r: manager_contract::PublicReadiness) -> Self {
        match r {
            manager_contract::PublicReadiness::Unavailable(reason) => Self::unavailable(reason),
            manager_contract::PublicReadiness::Ready(e) => Self {
                status: "ready",
                reason: None,
                checked_at: Some(e.checked_at.to_rfc3339()),
                expires_at: Some(e.expires_at.to_rfc3339()),
                evidence_digest: Some(e.evidence_digest),
                policy_digest: Some(e.policy_digest),
            },
        }
    }
}
#[derive(Serialize)]
pub struct ManagerRuntimeResponse {
    state: &'static str,
    contract_version: u32,
    supported: bool,
    runtime: Option<RuntimeResponse>,
    public_readiness: PublicReadinessResponse,
}
fn runtime_response(
    row: Option<crate::models::platform_bot_runtime_controls::PlatformBotRuntimeControls>,
) -> ManagerRuntimeResponse {
    use manager_contract::UnavailableReason;
    let reason = match &row {
        None => UnavailableReason::RuntimeUninitialized,
        Some(r) if r.retired => UnavailableReason::ManagerUnavailable,
        Some(r) if r.emergency_stopped => UnavailableReason::EmergencyStopped,
        Some(r) if !r.public_enabled => UnavailableReason::PublicDisabled,
        _ => UnavailableReason::PublicNotReady,
    };
    ManagerRuntimeResponse {
        state: "setup_incomplete",
        contract_version: manager_contract::CONTRACT_VERSION,
        supported: true,
        runtime: row.map(Into::into),
        public_readiness: PublicReadinessResponse::unavailable(reason),
    }
}

async fn runtime_status(
    db: &mongodb::Database,
    row: Option<crate::models::platform_bot_runtime_controls::PlatformBotRuntimeControls>,
) -> AppResult<ManagerRuntimeResponse> {
    let status = crate::services::platform_bot_status_service::view(db, row.as_ref()).await?;
    let readiness = if let Some(r) = &row {
        if status.state != "connection_repair_required"
            && !r.retired
            && !r.emergency_stopped
            && r.public_enabled
            && r.public_policy.is_some()
        {
            Some(
                crate::services::platform_public_policy::assess_enabled(
                    db,
                    &runtime_controls::policy(r),
                )
                .await?,
            )
        } else {
            None
        }
    } else {
        None
    };
    let activation = if let Some(row) = &row {
        db.collection::<crate::models::platform_public_contract::PublicDeploymentActivation>(
            crate::models::platform_public_contract::ACTIVATIONS,
        )
        .find_one(mongodb::bson::doc! {"_id": &row.platform_config_id})
        .await?
    } else {
        None
    };
    let mut response = runtime_response(row);
    if let Some(runtime) = &mut response.runtime {
        runtime.activated_policy_digest = activation.map(|active| active.policy_digest);
    }
    if let Some(readiness) = readiness {
        response.public_readiness = PublicReadinessResponse::from_readiness(readiness);
    }
    response.state = status.state;
    if status.state == "connection_repair_required" {
        response.public_readiness = PublicReadinessResponse::unavailable(
            manager_contract::UnavailableReason::TransportUnavailable,
        );
    } else if status.faq_ready {
        response.public_readiness.status = "faq_ready";
        response.public_readiness.reason = None;
    }
    Ok(response)
}

pub async fn get_runtime(
    State(state): State<AppState>,
    auth: AuthUser,
    Extension(_producers): Extension<ManagerProducers>,
    Path(id): Path<uuid::Uuid>,
) -> AppResult<(HeaderMap, Json<ManagerRuntimeResponse>)> {
    admin(&state, &auth).await?;
    configs::get(&state.db, &id.to_string()).await?;
    Ok((
        private_headers(),
        Json(
            runtime_status(
                &state.db,
                runtime_controls::get(&state.db, &id.to_string()).await?,
            )
            .await?,
        ),
    ))
}
pub async fn put_runtime(
    State(state): State<AppState>,
    auth: AuthUser,
    Extension(producers): Extension<ManagerProducers>,
    Path(id): Path<uuid::Uuid>,
    Json(input): Json<RuntimeInput>,
) -> AppResult<(HeaderMap, Json<ManagerRuntimeResponse>)> {
    admin(&state, &auth).await?;
    let row = runtime_controls::update_patch(
        &state.db,
        &audit_service::AuditActor::from_auth_user(&auth),
        state.audit_chain_hmac_key.as_ref().as_ref(),
        &id.to_string(),
        &input.patch(),
        producers.public.as_ref(),
    )
    .await?;
    Ok((
        private_headers(),
        Json(runtime_status(&state.db, Some(row)).await?),
    ))
}
pub async fn clear_runtime_emergency_stop(
    State(state): State<AppState>,
    auth: AuthUser,
    Extension(_producers): Extension<ManagerProducers>,
    Path(id): Path<uuid::Uuid>,
    Json(input): Json<ClearEmergencyInput>,
) -> AppResult<(HeaderMap, Json<ManagerRuntimeResponse>)> {
    admin(&state, &auth).await?;
    let row = runtime_controls::clear_emergency_stop(
        &state.db,
        &audit_service::AuditActor::from_auth_user(&auth),
        state.audit_chain_hmac_key.as_ref().as_ref(),
        &id.to_string(),
        &manager_contract::ExpectedRuntimeRevisions {
            manager_authority_epoch: input.expected_manager_authority_epoch,
            personal_control_revision: input.expected_personal_control_revision,
            public_policy_revision: input.expected_public_policy_revision,
        },
    )
    .await?;
    Ok((
        private_headers(),
        Json(runtime_status(&state.db, Some(row)).await?),
    ))
}
#[derive(Serialize)]
pub struct PublicProfileResponse {
    id: String,
    label: String,
}
#[derive(Serialize)]
pub struct PublicProfilesResponse {
    profiles: Vec<PublicProfileResponse>,
    default_profile: Option<String>,
    readiness: PublicReadinessResponse,
}
pub async fn discover_public_profiles(
    State(state): State<AppState>,
    auth: AuthUser,
    Extension(producers): Extension<ManagerProducers>,
    Path(id): Path<uuid::Uuid>,
    Json(input): Json<PublicReviewInput>,
) -> AppResult<(HeaderMap, Json<PublicProfilesResponse>)> {
    admin(&state, &auth).await?;
    let policy = runtime_controls::reviewed_policy(
        &state.db,
        &auth.user_id.to_string(),
        &id.to_string(),
        &input.patch(),
    )
    .await?;
    let discovery = producers
        .public
        .discover_public_profiles(&state.db, &policy)
        .await?;
    if discovery
        .default_profile
        .as_ref()
        .is_some_and(|id| !discovery.profiles.iter().any(|p| p.id == *id))
    {
        return Err(crate::errors::AppError::Internal(
            "Public profile discovery unavailable".into(),
        ));
    }
    Ok((
        private_headers(),
        Json(PublicProfilesResponse {
            profiles: discovery
                .profiles
                .into_iter()
                .map(|p| PublicProfileResponse {
                    id: p.id,
                    label: p.label,
                })
                .collect(),
            default_profile: discovery.default_profile,
            readiness: PublicReadinessResponse::from_readiness(discovery.readiness),
        }),
    ))
}
#[derive(Serialize)]
pub struct ReadinessResponse {
    public_readiness: PublicReadinessResponse,
}
pub async fn probe_public_readiness(
    State(state): State<AppState>,
    auth: AuthUser,
    Extension(producers): Extension<ManagerProducers>,
    Path(id): Path<uuid::Uuid>,
    Json(input): Json<PublicReviewInput>,
) -> AppResult<(HeaderMap, Json<ReadinessResponse>)> {
    admin(&state, &auth).await?;
    let policy = runtime_controls::reviewed_policy(
        &state.db,
        &auth.user_id.to_string(),
        &id.to_string(),
        &input.patch(),
    )
    .await?;
    let readiness = producers
        .public
        .probe_public_readiness(&state.db, &policy)
        .await?;
    Ok((
        private_headers(),
        Json(ReadinessResponse {
            public_readiness: PublicReadinessResponse::from_readiness(readiness),
        }),
    ))
}

#[cfg(test)]
mod runtime_input_tests {
    use super::*;
    use serde_json::json;
    fn base() -> serde_json::Value {
        json!({"initialize":true,"expected_profile_generation":7,"expected_manager_authority_epoch":0,"expected_personal_control_revision":0,"expected_public_policy_revision":0,"personal_enabled":true,"public_enabled":false})
    }
    #[tokio::test]
    async fn personal_only_status_reports_disabled_public_without_an_ai_blocker() {
        let (db, update, _) = crate::services::platform_personal_binding_service::tests::fixture(
            "personal_only_runtime_status",
        )
        .await;
        let runtime = runtime_controls::get(&db, &update.profile_id)
            .await
            .unwrap()
            .unwrap();
        assert!(runtime.personal_enabled && !runtime.public_enabled);
        let response =
            serde_json::to_value(runtime_status(&db, Some(runtime)).await.unwrap()).unwrap();
        assert_eq!(response["state"], "active");
        assert_eq!(response["runtime"]["personal_enabled"], true);
        assert_eq!(response["public_readiness"]["status"], "unavailable");
        assert_eq!(response["public_readiness"]["reason"], "public_disabled");
        db.drop().await.unwrap();
    }
    #[test]
    fn control_patches_reject_null_scalars_and_unknown_fields() {
        assert!(serde_json::from_value::<RuntimeInput>(base()).is_ok());
        for field in [
            "personal_enabled",
            "public_enabled",
            "emergency_stopped",
            "expected_profile_generation",
            "expected_manager_authority_epoch",
        ] {
            let mut body = base();
            body[field] = serde_json::Value::Null;
            assert!(
                serde_json::from_value::<RuntimeInput>(body).is_err(),
                "{field}"
            );
        }
        for (field, value) in [
            ("source", json!("other")),
            ("profile", serde_json::Value::Null),
            ("total_daily_credits", json!(1)),
            ("sender_daily_credits", serde_json::Value::Null),
        ] {
            let mut body = base();
            body["public_policy"] = json!({field:value});
            assert!(
                serde_json::from_value::<RuntimeInput>(body).is_err(),
                "{field}"
            );
        }
        let mut body = base();
        body["owner_user_id"] = json!("caller-selected");
        assert!(serde_json::from_value::<RuntimeInput>(body).is_err());
    }
    #[test]
    fn policy_omission_and_explicit_clear_remain_distinct() {
        assert!(matches!(
            serde_json::from_value::<RuntimeInput>(base())
                .unwrap()
                .patch()
                .public_policy,
            manager_contract::PolicyPatch::Unchanged
        ));
        let mut body = base();
        body["public_policy"] = serde_json::Value::Null;
        assert!(matches!(
            serde_json::from_value::<RuntimeInput>(body)
                .unwrap()
                .patch()
                .public_policy,
            manager_contract::PolicyPatch::Clear
        ));
    }
    #[test]
    fn absent_configuration_advertises_supported_without_inventing_versions() {
        let value = serde_json::to_value(runtime_response(None)).unwrap();
        assert_eq!(value["supported"], true);
        assert!(value["runtime"].is_null());
        assert_eq!(value["public_readiness"]["reason"], "runtime_uninitialized");
        assert!(value["public_readiness"]["evidence_digest"].is_null());
    }
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeTransportInput {
    Direct,
    Gateway,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeConversionRequest {
    pub channel_bot_id: uuid::Uuid,
    pub funding_owner_user_id: Option<uuid::Uuid>,
    pub transport: NativeTransportInput,
    pub recovery: Option<NativeSetupRecovery>,
    pub lifecycle_review: Option<NativeLifecycleReview>,
    pub replacement_connection_id: Option<uuid::Uuid>,
    pub public_key: Option<String>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeLifecycleReview {
    pub channel_bot_id: uuid::Uuid,
    pub profile_id: uuid::Uuid,
    pub generation: Option<i64>,
    pub reviewed_by: uuid::Uuid,
    pub snapshot_digest: String,
}
impl NativeLifecycleReview {
    fn into_service(self) -> crate::services::platform_entry_admin_service::NativeLifecycleReview {
        crate::services::platform_entry_admin_service::NativeLifecycleReview {
            channel_bot_id: self.channel_bot_id.to_string(),
            profile_id: self.profile_id.to_string(),
            generation: self.generation,
            reviewed_by: self.reviewed_by.to_string(),
            snapshot_digest: self.snapshot_digest,
        }
    }
}
pub async fn review_native_lifecycle(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(bot): Path<uuid::Uuid>,
) -> AppResult<(HeaderMap, Json<NativeLifecycleReview>)> {
    admin(&state, &auth).await?;
    let review = crate::services::platform_entry_admin_service::review_lifecycle(
        &state,
        &auth.user_id.to_string(),
        &bot.to_string(),
    )
    .await?;
    Ok((
        private_headers(),
        Json(NativeLifecycleReview {
            channel_bot_id: bot,
            profile_id: uuid::Uuid::parse_str(&review.profile_id).map_err(|_| {
                crate::errors::AppError::Internal("Invalid native profile identity".into())
            })?,
            generation: review.generation,
            reviewed_by: auth.user_id,
            snapshot_digest: review.snapshot_digest,
        }),
    ))
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeSetupRecovery {
    Discord {
        attempt_id: String,
        application_id: String,
        command_id: Option<String>,
        observed_schema_digest: Option<String>,
        schema_digest: String,
        requested_by: String,
        channel_bot_id: String,
        credential_fingerprint: String,
    },
    Gateway {
        record_id: String,
        attempt_id: String,
        revision: i64,
        request_hash: String,
        credential_fingerprint: String,
        remote_channel_id: Option<String>,
        remote_version: Option<i64>,
        remote_ready: bool,
    },
}
impl From<crate::services::platform_entry_gateway_service::GatewayRecoveryReview>
    for NativeSetupRecovery
{
    fn from(row: crate::services::platform_entry_gateway_service::GatewayRecoveryReview) -> Self {
        Self::Gateway {
            record_id: row.record_id,
            attempt_id: row.attempt_id,
            revision: row.revision,
            request_hash: row.request_hash,
            credential_fingerprint: row.credential_fingerprint,
            remote_channel_id: row.remote_channel_id,
            remote_version: row.remote_version,
            remote_ready: row.remote_ready,
        }
    }
}
impl From<crate::services::platform_entry_discord_installation::RecoveryReview>
    for NativeSetupRecovery
{
    fn from(row: crate::services::platform_entry_discord_installation::RecoveryReview) -> Self {
        Self::Discord {
            attempt_id: row.attempt_id,
            application_id: row.application_id,
            command_id: row.command_id,
            observed_schema_digest: row.observed_schema_digest,
            schema_digest: row.schema_digest,
            requested_by: row.requested_by,
            channel_bot_id: row.channel_bot_id,
            credential_fingerprint: row.credential_fingerprint,
        }
    }
}
#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum NativeConversionResponse {
    Converted { profile: ConfigResponse },
    Recovery { review: NativeSetupRecovery },
}
#[derive(Serialize)]
pub struct NativePlatformTransports {
    pub platform: &'static str,
    pub transports: Vec<&'static str>,
}
pub async fn native_transports(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<(HeaderMap, Json<Vec<NativePlatformTransports>>)> {
    admin(&state, &auth).await?;
    use crate::models::platform_entry_identity::EntryPlatform::*;
    Ok((
        private_headers(),
        Json(
            [Discord, Slack, Whatsapp, Lark, Feishu, X]
                .into_iter()
                .map(|platform| NativePlatformTransports {
                    platform: crate::services::platform_entry_native::platform_name(platform),
                    transports:
                        crate::services::platform_entry_gateway_service::supported_transports(
                            platform,
                        )
                        .to_vec(),
                })
                .collect(),
        ),
    ))
}
pub async fn convert_native(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<NativeConversionRequest>,
) -> AppResult<(HeaderMap, Json<NativeConversionResponse>)> {
    admin(&state, &auth).await?;
    if let Some(review) = body.lifecycle_review {
        if review.channel_bot_id != body.channel_bot_id {
            return Err(crate::errors::AppError::ValidationError(
                "Native review target changed".into(),
            ));
        }
        let review = review.into_service();
        let connection = body.replacement_connection_id.map(|id| id.to_string());
        crate::services::platform_entry_admin_service::reserve_reviewed_update(
            &state,
            &auth.user_id.to_string(),
            &body
                .funding_owner_user_id
                .unwrap_or(auth.user_id)
                .to_string(),
            match body.transport {
                NativeTransportInput::Direct => {
                    crate::services::platform_entry_admin_service::NativeTransportChoice::Direct
                }
                NativeTransportInput::Gateway => {
                    crate::services::platform_entry_admin_service::NativeTransportChoice::Gateway
                }
            },
            &review,
            connection.as_deref(),
            body.public_key.as_deref(),
        )
        .await?;
    } else if body.replacement_connection_id.is_some() || body.public_key.is_some() {
        return Err(crate::errors::AppError::ValidationError(
            "Review the exact native setup before replacing credentials".into(),
        ));
    }
    let mut discord = None;
    let mut gateway = None;
    if let Some(review) = body.recovery {
        // Reviews are metadata, bounded before comparison with the retained
        // server snapshot. None of these values supplies provider authority.
        let encoded = serde_json::to_vec(&review)
            .map_err(|_| crate::errors::AppError::ValidationError("Invalid setup review".into()))?;
        if encoded.len() > 4096 {
            return Err(crate::errors::AppError::ValidationError(
                "Invalid setup review".into(),
            ));
        }
        match review {
            NativeSetupRecovery::Discord {
                attempt_id,
                application_id,
                command_id,
                observed_schema_digest,
                schema_digest,
                requested_by,
                channel_bot_id,
                credential_fingerprint,
            } => {
                discord = Some(
                    crate::services::platform_entry_discord_installation::RecoveryReview {
                        attempt_id,
                        application_id,
                        command_id,
                        observed_schema_digest,
                        schema_digest,
                        requested_by,
                        channel_bot_id,
                        credential_fingerprint,
                    },
                );
            }
            NativeSetupRecovery::Gateway {
                record_id,
                attempt_id,
                revision,
                request_hash,
                credential_fingerprint,
                remote_channel_id,
                remote_version,
                remote_ready,
            } => {
                gateway = Some(
                    crate::services::platform_entry_gateway_service::GatewayRecoveryReview {
                        record_id,
                        attempt_id,
                        revision,
                        request_hash,
                        credential_fingerprint,
                        remote_channel_id,
                        remote_version,
                        remote_ready,
                    },
                );
            }
        }
    }
    let transport = match body.transport {
        NativeTransportInput::Direct => {
            crate::services::platform_entry_admin_service::NativeTransportChoice::Direct
        }
        NativeTransportInput::Gateway => {
            crate::services::platform_entry_admin_service::NativeTransportChoice::Gateway
        }
    };
    let outcome = crate::services::platform_entry_admin_service::convert(
        &state,
        &auth.user_id.to_string(),
        &body.channel_bot_id.to_string(),
        &body
            .funding_owner_user_id
            .unwrap_or(auth.user_id)
            .to_string(),
        transport,
        discord.as_ref(),
        gateway.as_ref(),
    )
    .await?;
    use crate::services::platform_entry_admin_service::NativeConversionOutcome;
    let response = match outcome {
        NativeConversionOutcome::Converted(row) => {
            audit_service::log_for_user(
                state.db.clone(),
                &auth,
                "platform_bot_converted",
                Some(serde_json::json!({"platform_config_id":row.id,"generation":row.generation})),
            );
            NativeConversionResponse::Converted {
                profile: config_response(&state.db, *row).await?,
            }
        }
        NativeConversionOutcome::DiscordRecovery(review) => NativeConversionResponse::Recovery {
            review: review.into(),
        },
        NativeConversionOutcome::GatewayRecovery(review) => NativeConversionResponse::Recovery {
            review: review.into(),
        },
    };
    Ok((private_headers(), Json(response)))
}
pub async fn inspect_native_gateway(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(bot): Path<uuid::Uuid>,
) -> AppResult<(HeaderMap, Json<NativeSetupRecovery>)> {
    admin(&state, &auth).await?;
    Ok((
        private_headers(),
        Json(
            crate::services::platform_entry_admin_service::inspect_gateway(
                &state,
                &auth.user_id.to_string(),
                &bot.to_string(),
            )
            .await?
            .into(),
        ),
    ))
}

#[derive(Serialize)]
pub struct ConversionTargetResponse {
    id: String,
    platform: String,
    label: String,
    platform_bot_username: String,
    credential_source: String,
}
#[derive(Serialize)]
pub struct ConversionTargetsResponse {
    bots: Vec<ConversionTargetResponse>,
    next_before_id: Option<String>,
}
pub async fn conversion_targets(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<ListQuery>,
) -> AppResult<(HeaderMap, Json<ConversionTargetsResponse>)> {
    admin(&state, &auth).await?;
    let (bots, next_before_id) = configs::conversion_targets(
        &state.db,
        query.before_id.map(|id| id.to_string()).as_deref(),
    )
    .await?;
    Ok((
        private_headers(),
        Json(ConversionTargetsResponse {
            next_before_id,
            bots: bots
                .into_iter()
                .map(|bot| ConversionTargetResponse {
                    id: bot.id,
                    platform: bot.platform,
                    label: bot.label,
                    platform_bot_username: bot.platform_bot_username,
                    credential_source: bot.credential_source,
                })
                .collect(),
        }),
    ))
}
