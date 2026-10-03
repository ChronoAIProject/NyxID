//! HTTP/native adapters for agent guidance. All Ornn egress uses the normal proxy.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::AuthUser,
    services::{
        agent_skill_service as skills,
        assistant_acknowledgement_service::{self as acks, ChatAuthority},
        assistant_team_service as team,
    },
};
use axum::{
    Json,
    body::{Body, to_bytes},
    extract::{Path, Query, State},
    http::Request,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone)]
pub(crate) struct OrnnSkillRead;
struct Reader<'a> {
    state: &'a AppState,
    person: &'a str,
    thread_key: Option<&'a str>,
    scopes: Option<&'a crate::models::agent_operation_scope::OperationScopes>,
}
#[async_trait::async_trait]
impl skills::OrnnReader for Reader<'_> {
    async fn get(&self, path: &str) -> AppResult<Vec<u8>> {
        // A proxy request is a substantial state machine. Poll it as its own
        // task so nested native MCP dispatch does not accumulate its debug
        // stack frames. Cancellation/timeout must also cancel that task.
        let state = self.state.clone();
        let person = self.person.to_owned();
        let path = path.to_owned();
        let thread_key = self.thread_key.map(str::to_owned);
        let scopes = self.scopes.cloned().unwrap_or_default();
        let task = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(async move {
            Box::pin(fetch_ornn(&state, &person, &path, thread_key, scopes)).await
        }));
        tokio::time::timeout(std::time::Duration::from_secs(30), task)
            .await
            .map_err(|_| {
                AppError::Forbidden("Ornn is unavailable; skill read timed out, retry later".into())
            })?
            .map_err(|_| {
                AppError::Forbidden("Ornn is unavailable; retry the skill read later".into())
            })?
    }
}

async fn fetch_ornn(
    state: &AppState,
    person: &str,
    path: &str,
    thread_key: Option<String>,
    scopes: crate::models::agent_operation_scope::OperationScopes,
) -> AppResult<Vec<u8>> {
    // Fixed catalog selection; no caller-supplied destination, method or headers.
    let service = state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .find_one(mongodb::bson::doc! {"slug":"ornn-api","is_active":true})
        .await?
        .ok_or_else(|| {
            AppError::Forbidden("Ornn is unavailable; connect your Ornn access and retry".into())
        })?;
    let mut auth = super::assistant_team::owner_auth(person)?;
    if let Some(key) = thread_key {
        // Native reads are agent requests, never browser-session bypasses of
        // approval policy. The key also binds live scope/guest/automation checks.
        // Authority here is limited to the adapter's fixed Ornn reads; runtime
        // package reads have already resolved an attached pin for this agent.
        auth.auth_method = crate::mw::auth::AuthMethod::ApiKey;
        auth.scope = "proxy".into();
        auth.api_key_id = Some(key);
    }
    auth.assistant_operation_scopes = scopes;
    let mut request = Request::builder()
        .uri(format!("/api/v1/proxy/{}{path}", service.id))
        .body(Body::empty())
        .map_err(|_| AppError::ValidationError("Invalid skill path".into()))?;
    request
        .extensions_mut()
        .insert(super::assistant_nyxagent::SERVER_TURN_POLICY);
    request.extensions_mut().insert(OrnnSkillRead);
    let mut slug = String::new();
    let response = Box::pin(super::proxy::proxy_request_inner(
        state,
        &auth,
        &service.id,
        path.split('?').next().unwrap_or(path),
        request,
        &mut slug,
    ))
    .await
    .map_err(|error| match error {
        AppError::ApiKeyScopeForbidden(_) => error,
        _ => AppError::Forbidden(
            "Skill unavailable through your Ornn access; check the connection and retry".into(),
        ),
    })?;
    if !response.status().is_success() {
        return Err(AppError::Forbidden(
            "Skill unavailable or not visible through your Ornn access".into(),
        ));
    }
    to_bytes(response.into_body(), skills::MAX_ARCHIVE)
        .await
        .map(|b| b.to_vec())
        .map_err(|_| {
            AppError::ValidationError(
                "Ornn response exceeded the skill size limit or was interrupted".into(),
            )
        })
}

#[derive(Deserialize)]
pub struct CatalogQuery {
    #[serde(default)]
    q: String,
    #[serde(default = "first_page")]
    page: u32,
    skill: Option<String>,
    version: Option<String>,
}
fn first_page() -> u32 {
    1
}
pub async fn catalog(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<CatalogQuery>,
) -> AppResult<Json<Value>> {
    let owner = auth.user_id.to_string();
    crate::services::assistant_nyxagent::require_enabled(&state.db, &owner).await?;
    let reader = Reader {
        state: &state,
        person: &owner,
        thread_key: None,
        scopes: None,
    };
    let value = match (query.skill, query.version) {
        (Some(id), Some(version)) => json!(skills::preview(&reader, &id, &version).await?),
        (Some(id), None) => skills::versions(&reader, &id, query.page).await?,
        (None, _) => skills::search(&reader, &query.q, query.page).await?,
    };
    Ok(Json(value))
}
pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<skills::SkillsResponse>> {
    let owner = auth.user_id.to_string();
    crate::services::assistant_nyxagent::require_enabled(&state.db, &owner).await?;
    Ok(Json(team::agent(&state.db, &owner, &id).await?.into()))
}
pub async fn set(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(selection): Json<skills::Selection>,
) -> AppResult<Json<skills::SkillsResponse>> {
    let owner = auth.user_id.to_string();
    crate::services::assistant_nyxagent::require_enabled(&state.db, &owner).await?;
    Ok(Json(
        Box::pin(skills::set(
            &state.db,
            &Reader {
                state: &state,
                person: &owner,
                thread_key: None,
                scopes: None,
            },
            &owner,
            &id,
            &selection,
            true,
        ))
        .await?,
    ))
}
fn arg<'a>(args: &'a Value, key: &str) -> &'a str {
    args[key].as_str().unwrap_or_default()
}

pub(crate) async fn dispatch(
    state: &AppState,
    chat: &ChatAuthority,
    name: &str,
    args: &Value,
) -> AppResult<(Value, bool)> {
    if chat.guest {
        return Err(AppError::Forbidden(
            "Only owner turns may access attached skills".into(),
        ));
    }
    let reader = Reader {
        state,
        person: &chat.user_id,
        thread_key: Some(&chat.api_key_id),
        scopes: None,
    };
    let db = &state.db;
    let value = match name {
        "search_agent_skills" => {
            skills::search(
                &reader,
                arg(args, "query"),
                args["page"].as_u64().unwrap_or(1) as u32,
            )
            .await?
        }
        "agent_skill_versions" => {
            skills::versions(
                &reader,
                arg(args, "skill"),
                args["page"].as_u64().unwrap_or(1) as u32,
            )
            .await?
        }
        "preview_agent_skill" => {
            json!(skills::preview(&reader, arg(args, "skill"), arg(args, "version")).await?)
        }
        "skill_read" => {
            let agent = team::agent(db, &chat.user_id, &chat.agent_id).await?;
            crate::services::org_agent_service::require_use(db, &chat.user_id, &agent).await?;
            if agent.destroyed_at.is_some() {
                return Err(AppError::Forbidden("Agent is destroyed".into()));
            }
            let reader = Reader {
                state,
                person: &chat.user_id,
                thread_key: Some(&chat.api_key_id),
                scopes: Some(&agent.operation_scopes),
            };
            skills::read(
                &reader,
                &agent,
                arg(args, "skill"),
                args["dependency"].as_str(),
                args["path"].as_str().unwrap_or("SKILL.md"),
                args["offset"].as_u64().unwrap_or(0) as usize,
            )
            .await?
        }
        _ => {
            let target = args["agent"].as_str().unwrap_or("nyxbot");
            let agent = if uuid::Uuid::parse_str(target).is_ok() {
                team::agent(db, &chat.user_id, target).await?
            } else {
                super::assistant_team::target_agent(state, &chat.user_id, Some(target)).await?
            };
            if !chat.is_orchestrator() && (agent.id != chat.agent_id || name == "set_agent_skills")
            {
                return Err(AppError::Forbidden("Specialists may read or request only their own skills; only NyxBot manages skills".into()));
            }
            if name == "get_agent_skills" {
                return Ok((json!(skills::SkillsResponse::from(agent)), false));
            }
            if name == "set_agent_skills" && target != agent.id {
                return Err(AppError::ValidationError(
                    "Use the immutable agent ID from get_agent_skills when setting skills".into(),
                ));
            }
            if agent.destroyed_at.is_some() {
                return Err(AppError::Forbidden("Agent is destroyed".into()));
            }
            if name == "request_agent_skills" && chat.is_orchestrator() {
                return Err(AppError::ValidationError(
                    "NyxBot manages skills with set_agent_skills".into(),
                ));
            }
            if name == "request_agent_skills" && args.get("skill").is_some() {
                if args.get("selection").is_some() {
                    return Err(AppError::ValidationError(
                        "Request a skill or a pinned selection, not both".into(),
                    ));
                }
                let summary = format!(
                    "Skill {} version {} for agent {}. NyxBot must resolve and preview exact pins before requesting the owner's attachment card.",
                    arg(args, "skill"),
                    args["version"]
                        .as_str()
                        .unwrap_or("to be selected by the owner"),
                    agent.id
                );
                let (card, created) = Box::pin(acks::request_tracked(
                    db,
                    chat,
                    acks::Request {
                        kind: "skills",
                        service: None,
                        tool: None,
                        arguments: Some(args),
                        summary: &summary,
                        platform: false,
                    },
                ))
                .await?;
                if created {
                    Box::pin(super::assistant_team::permission_requested(
                        state, chat, &card,
                    ))
                    .await;
                }
                return Ok((acks::refusal(&card), true));
            }
            if name == "set_agent_skills" {
                crate::services::org_agent_service::require_maintain(db, &chat.user_id, &agent)
                    .await?;
            }
            let selection: skills::Selection = serde_json::from_value(args["selection"].clone())
                .map_err(|_| AppError::ValidationError("Invalid skill selection".into()))?;
            skills::validate(&selection)?;
            if agent.skills_revision != selection.expected_revision {
                return Err(AppError::Conflict(
                    "Skills changed; reload the current revision".into(),
                ));
            }
            let summary = format!(
                "Skills for {} at revision {}: {}",
                agent.name,
                selection.expected_revision,
                selection
                    .skills
                    .iter()
                    .map(|s| format!("{} @ {}", s.skill_id, s.version))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            if name == "request_agent_skills" {
                let (card, created) = Box::pin(acks::request_tracked(
                    db,
                    chat,
                    acks::Request {
                        kind: "skills",
                        service: None,
                        tool: None,
                        arguments: Some(&args["selection"]),
                        summary: &summary,
                        platform: false,
                    },
                ))
                .await?;
                if created {
                    Box::pin(super::assistant_team::permission_requested(
                        state, chat, &card,
                    ))
                    .await;
                }
                return Ok((acks::refusal(&card), true));
            }
            let confirmed = if let Some(id) = args["acknowledgement_id"].as_str() {
                if !acks::consume_action(db, chat, id, "nyxid__set_agent_skills", args).await? {
                    return Ok((json!({"error":"acknowledgement_invalid"}), true));
                }
                true
            } else {
                false
            };
            if !confirmed
                && (skills::needs_confirmation(&agent, &selection)
                    || acks::webhook_confirmation_required(chat, false, false))
            {
                return super::assistant_team::operation_owner_card(
                    db,
                    chat,
                    "nyxid__set_agent_skills",
                    args,
                    &summary,
                )
                .await;
            }
            json!(
                Box::pin(skills::set(
                    db,
                    &reader,
                    &chat.user_id,
                    &agent.id,
                    &selection,
                    confirmed
                ))
                .await?
            )
        }
    };
    Ok((value, false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::{
        agent_skill_service::OrnnReader, assistant_authority_tests::orchestrator_fixture,
    };
    use mongodb::bson::doc;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

    #[tokio::test]
    async fn agent_skills_ornn_requires_live_person_identity_and_refuses_master_credentials() {
        let f = orchestrator_fixture("skill_ornn_identity").await;
        let upstream = MockServer::start().await;
        let mut service = crate::test_utils::test_auto_connected_catalog_service();
        service.slug = "ornn-api".into();
        service.base_url = upstream.uri();
        service.identity_propagation_mode = "jwt".into();
        let collection = f
            .state
            .db
            .collection::<crate::models::downstream_service::DownstreamService>(
                crate::models::downstream_service::COLLECTION_NAME,
            );
        collection.insert_one(&service).await.unwrap();
        Mock::given(path("/api/v1/skill-search"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"data":{"items":[],"totalPages":0}})),
            )
            .mount(&upstream)
            .await;
        let reader = Reader {
            state: &f.state,
            person: &f.owner,
            thread_key: None,
            scopes: None,
        };
        assert!(skills::search(&reader, "private", 1).await.is_ok());
        let requests = upstream.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].headers.contains_key("x-nyxid-identity-token"));
        // No failure formatter ever receives the signed token.
        let token = requests[0]
            .headers
            .get("x-nyxid-identity-token")
            .unwrap()
            .to_str()
            .unwrap();
        let claims = base64::Engine::decode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            token.split('.').nth(1).unwrap(),
        )
        .unwrap();
        let claims: Value = serde_json::from_slice(&claims).unwrap();
        assert!(claims["sub"].as_str() == Some(f.owner.as_str()));
        collection
            .update_one(
                doc! {"_id":&service.id},
                doc! {"$set":{"identity_propagation_mode":"none"}},
            )
            .await
            .unwrap();
        assert!(skills::search(&reader, "private", 1).await.is_err());
        assert_eq!(upstream.received_requests().await.unwrap().len(), 1);
        let encrypted = f
            .state
            .encryption_keys
            .encrypt(b"test-only-ornn-fixture")
            .await
            .unwrap();
        collection.update_one(doc!{"_id":&service.id},doc!{"$set":{"identity_propagation_mode":"jwt","auth_method":"bearer","service_category":"internal","credential_encrypted":mongodb::bson::Binary{subtype:mongodb::bson::spec::BinarySubtype::Generic,bytes:encrypted}}}).await.unwrap();
        assert!(skills::search(&reader, "private", 1).await.is_err());
        assert_eq!(upstream.received_requests().await.unwrap().len(), 1);
        // Missing principal must never become an anonymous Ornn request.
        collection
            .update_one(
                doc! {"_id":&service.id},
                doc! {"$set":{"auth_method":"none"}},
            )
            .await
            .unwrap();
        let missing = uuid::Uuid::new_v4().to_string();
        assert!(
            Reader {
                state: &f.state,
                person: &missing,
                thread_key: None,
                scopes: None,
            }
            .get("/api/v1/skill-search")
            .await
            .is_err()
        );
        assert_eq!(upstream.received_requests().await.unwrap().len(), 1);
        // Ornn visibility refusal is preserved without returning upstream bodies.
        upstream.reset().await;
        Mock::given(path("/api/v1/skills/128393f3-d528-4ce2-b197-f1b13cb8fd5b"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&upstream)
            .await;
        assert!(
            skills::preview(&reader, "128393f3-d528-4ce2-b197-f1b13cb8fd5b", "1.0")
                .await
                .is_err()
        );
        // Native reads retain runtime approvals; browser preview remains an
        // explicit human read. No upstream bytes flow before the owner decides.
        upstream.reset().await;
        Mock::given(path("/api/v1/skill-search"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"data":{"items":[],"totalPages":0}})),
            )
            .mount(&upstream)
            .await;
        let now = chrono::Utc::now();
        f.state
            .db
            .collection::<crate::models::service_approval_config::ServiceApprovalConfig>(
                crate::models::service_approval_config::COLLECTION_NAME,
            )
            .insert_one(
                crate::models::service_approval_config::ServiceApprovalConfig {
                    id: uuid::Uuid::new_v4().to_string(),
                    user_id: f.owner.clone(),
                    service_id: service.id.clone(),
                    service_name: service.name.clone(),
                    approval_required: true,
                    approval_mode: crate::models::service_approval_config::ApprovalMode::PerRequest,
                    rules: vec![],
                    default_effect: None,
                    created_at: now,
                    updated_at: now,
                },
            )
            .await
            .unwrap();
        assert!(skills::search(&reader, "human", 1).await.is_ok());
        let before = upstream.received_requests().await.unwrap().len();
        let state = f.state.clone();
        let owner = f.owner.clone();
        let key = f.chat.api_key_id.clone();
        let task = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(async move {
            skills::search(
                &Reader {
                    state: &state,
                    person: &owner,
                    thread_key: Some(&key),
                    scopes: None,
                },
                "native",
                1,
            )
            .await
        }));
        let pending = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Some(row) = f
                    .state
                    .db
                    .collection::<mongodb::bson::Document>(
                        crate::models::approval_request::COLLECTION_NAME,
                    )
                    .find_one(
                        doc! {"user_id": &f.owner, "service_id": &service.id, "status": "pending"},
                    )
                    .await
                    .unwrap()
                {
                    break row;
                }
                assert!(
                    !task.is_finished(),
                    "Native read ended before asking for approval"
                );
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("native approval request");
        assert_eq!(pending.get_str("requester_type").unwrap(), "api_key");
        assert_eq!(upstream.received_requests().await.unwrap().len(), before);
        drop(task);
        f.state.db.drop().await.unwrap();
    }
}
