//! Human-only personal consent endpoints. Part 5 mounts these after auth guards.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::platform_link_intent::{ChallengeState, PlatformLinkChallenge},
    mw::auth::AuthUser,
    services::{
        platform_link_intent_service as proofs, platform_personal_binding_service as bindings,
    },
};
use axum::{
    Extension, Json,
    extract::{ConnectInfo, Path, Query, State},
    http::{HeaderMap, header},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use zeroize::Zeroizing;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntentInput {
    pub intent_token: Zeroizing<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChallengeInput {
    pub challenge_id: uuid::Uuid,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApproveInput {
    pub challenge_id: uuid::Uuid,
    pub review_digest: String,
    #[serde(default)]
    pub replace_existing: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevokeInput {
    pub generation: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListInput {
    pub before_id: Option<uuid::Uuid>,
}
#[derive(Clone, Serialize)]
pub struct ManagerResponse {
    pub id: String,
    pub username: Option<String>,
    pub display_name: String,
}
#[derive(Serialize)]
pub struct SenderResponse {
    pub id: String,
    pub display_name: Option<String>,
}
#[derive(Serialize)]
pub struct PreviewResponse {
    pub chat: ChatResponse,
    pub purpose: &'static str,
    pub status: &'static str,
    pub manager: ManagerResponse,
    pub sender: SenderResponse,
    pub expires_at: String,
    pub invitation_required: bool,
    pub requires_same_chat_confirmation: bool,
}
#[derive(Serialize)]
pub struct AccountResponse {
    pub id: String,
    pub display_name: Option<String>,
}
#[derive(Serialize)]
pub struct Disclosures {
    pub telegram_transport: bool,
    pub existing_nyxbot_access: bool,
    pub can_approve_sensitive_actions: bool,
    pub started_work_may_continue_after_unlink: bool,
}
#[derive(Serialize)]
pub struct ReviewResponse {
    pub chat: ChatResponse,
    pub challenge_id: String,
    pub status: &'static str,
    pub expires_at: String,
    pub review_digest: String,
    pub account: AccountResponse,
    pub permission_mode: String,
    pub billing: &'static str,
    pub replaces_existing_binding: bool,
    pub disclosures: Disclosures,
}
#[derive(Serialize)]
pub struct BindingResponse {
    pub id: String,
    pub generation: i64,
}
#[derive(Serialize)]
pub struct StatusResponse {
    pub chat: ChatResponse,
    pub challenge_id: String,
    pub status: &'static str,
    pub expires_at: String,
    pub binding: Option<BindingResponse>,
    pub confirmation_command: Option<Zeroizing<String>>,
}

#[derive(Serialize)]
pub struct ChatResponse {
    pub platform: &'static str,
    pub display_name: &'static str,
    pub login_command: &'static str,
    pub logout_command: &'static str,
    pub status_command: &'static str,
    pub help_command: &'static str,
    pub question_instruction: &'static str,
    pub transport_disclosure: &'static str,
}
fn chat(
    entry: &crate::models::platform_entry_projection::StoredEntryIdentity,
) -> AppResult<ChatResponse> {
    let text = crate::services::platform_chat_presentation::ChatPresentation::for_entry(entry)?;
    Ok(ChatResponse {
        platform: text.platform(),
        display_name: text.name(),
        login_command: text.login(),
        logout_command: text.logout(),
        status_command: text.status(),
        help_command: text.help(),
        question_instruction: text.question_instruction(),
        transport_disclosure: text.transport_disclosure(),
    })
}

fn private_headers() -> HeaderMap {
    HeaderMap::from_iter([
        (header::CACHE_CONTROL, "no-store".parse().unwrap()),
        (header::REFERRER_POLICY, "no-referrer".parse().unwrap()),
    ])
}
async fn manager(
    db: &mongodb::Database,
    profile_id: &str,
    manager_bot_id: &str,
    entry: &crate::models::platform_entry_projection::StoredEntryIdentity,
    required: bool,
) -> AppResult<ManagerResponse> {
    if let crate::models::platform_entry_projection::StoredEntryIdentity::NativeV2 {
        platform_entry,
    } = entry
    {
        let display =
            bindings::native_manager_display(db, profile_id, manager_bot_id, platform_entry)
                .await?;
        if let Some((username, display_name)) = display {
            return Ok(ManagerResponse {
                id: profile_id.into(),
                username,
                display_name,
            });
        }
        if required {
            return Err(bindings::unavailable());
        }
        return Ok(ManagerResponse {
            id: profile_id.into(),
            username: None,
            display_name: format!("{} bot {}", chat(entry)?.display_name, manager_bot_id),
        });
    }
    if let Some(display) = bindings::manager_display(db, profile_id, manager_bot_id).await? {
        return Ok(ManagerResponse {
            id: profile_id.to_owned(),
            display_name: format!("@{}", display.username),
            username: Some(display.username),
        });
    }
    if required {
        return Err(bindings::unavailable());
    }
    // Retired inventory can still identify its exact historical bot numerically.
    Ok(ManagerResponse {
        id: profile_id.to_owned(),
        username: None,
        display_name: format!("Telegram bot {manager_bot_id}"),
    })
}
fn project_status(row: PlatformLinkChallenge) -> AppResult<StatusResponse> {
    Ok(StatusResponse {
        chat: chat(&row.entry)?,
        challenge_id: row.id,
        status: match row.state {
            ChallengeState::Review => "review",
            ChallengeState::AwaitingChatConfirmation => "awaiting_chat_confirmation",
            ChallengeState::Confirmed => "confirmed",
            ChallengeState::Denied => "denied",
            ChallengeState::Cancelled => "cancelled",
            ChallengeState::Expired => "expired",
        },
        expires_at: row.expires_at.to_rfc3339(),
        binding: row.completion.map(|b| BindingResponse {
            id: b.binding_id,
            generation: b.binding_generation,
        }),
        confirmation_command: None,
    })
}

pub async fn preview(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    Json(input): Json<IntentInput>,
) -> AppResult<(HeaderMap, Json<PreviewResponse>)> {
    let ip = super::login_client_context::resolve_client_ip(&headers, peer, &state)?;
    if !crate::services::coordination_service::RateWindowStore::admit(
        &state.db,
        "personal_intent_preview_ip",
        &ip.to_string(),
        30,
        std::time::Duration::from_secs(60),
    )
    .await?
    .allowed
    {
        return Err(AppError::RateLimited);
    }
    let row = proofs::preview(
        &state.db,
        state.auth_device_hmac_key.as_slice(),
        &input.intent_token,
    )
    .await?;
    Ok((
        private_headers(),
        Json(PreviewResponse {
            chat: chat(&row.entry)?,
            purpose: "personal_nyxbot",
            status: "pending",
            manager: manager(
                &state.db,
                &row.platform_config_id,
                &row.manager_bot_id,
                &row.entry,
                true,
            )
            .await?,
            sender: SenderResponse {
                id: crate::services::platform_entry_scope::actor_string(&row.sender),
                display_name: None,
            },
            expires_at: row.expires_at.to_rfc3339(),
            invitation_required: crate::services::feature_flag_service::invitation_code_required(
                &state.db,
            )
            .await?,
            requires_same_chat_confirmation: true,
        }),
    ))
}

pub async fn review(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<IntentInput>,
) -> AppResult<(HeaderMap, Json<ReviewResponse>)> {
    super::login_client_context::require_first_party_human(&auth)?;
    let actor = auth.user_id.to_string();
    let row = proofs::review(
        &state.db,
        state.auth_device_hmac_key.as_slice(),
        &actor,
        &input.intent_token,
    )
    .await?;
    let user = state
        .db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .find_one(bson::doc! {"_id":&actor,"is_active":true,"$or":[{"user_type":"person"},{"user_type":{"$exists":false}}]})
        .await?
        .ok_or_else(bindings::unavailable)?;
    let replaces = row.previous_binding.as_ref().is_some_and(|b| {
        b.state == crate::models::platform_personal_binding::PersonalBindingState::Active
            && b.user_id != actor
    });
    Ok((
        private_headers(),
        Json(ReviewResponse {
            chat: chat(&row.entry)?,
            challenge_id: row.id,
            status: "review",
            expires_at: row.expires_at.to_rfc3339(),
            review_digest: row.review_digest,
            account: AccountResponse {
                id: actor,
                display_name: user.display_name,
            },
            permission_mode: row.reviewed_access_mode,
            billing: "personal",
            replaces_existing_binding: replaces,
            disclosures: Disclosures {
                telegram_transport: matches!(
                    &row.entry,
                    crate::models::platform_entry_projection::StoredEntryIdentity::TelegramV1 { .. }
                ),
                existing_nyxbot_access: true,
                can_approve_sensitive_actions: true,
                started_work_may_continue_after_unlink: true,
            },
        }),
    ))
}

pub async fn approve(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<ApproveInput>,
) -> AppResult<(HeaderMap, Json<StatusResponse>)> {
    super::login_client_context::require_first_party_human(&auth)?;
    let result = proofs::approve(
        &state.db,
        state.auth_device_hmac_key.as_slice(),
        &auth.user_id.to_string(),
        &input.challenge_id.to_string(),
        &input.review_digest,
        input.replace_existing,
    )
    .await?;
    let mut response = project_status(result.challenge)?;
    response.confirmation_command = result.confirmation_command;
    Ok((private_headers(), Json(response)))
}

pub async fn status(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<ChallengeInput>,
) -> AppResult<(HeaderMap, Json<StatusResponse>)> {
    super::login_client_context::require_first_party_human(&auth)?;
    Ok((
        private_headers(),
        Json(project_status(
            proofs::status(
                &state.db,
                &auth.user_id.to_string(),
                &input.challenge_id.to_string(),
            )
            .await?,
        )?),
    ))
}

#[derive(Serialize)]
pub struct DeniedResponse {
    pub status: &'static str,
}
pub async fn deny(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<ChallengeInput>,
) -> AppResult<(HeaderMap, Json<DeniedResponse>)> {
    super::login_client_context::require_first_party_human(&auth)?;
    proofs::deny(
        &state.db,
        &auth.user_id.to_string(),
        &input.challenge_id.to_string(),
    )
    .await?;
    Ok((private_headers(), Json(DeniedResponse { status: "denied" })))
}

#[derive(Serialize)]
pub struct OwnedBindingResponse {
    pub chat: ChatResponse,
    pub id: String,
    pub generation: i64,
    pub manager: ManagerResponse,
    pub sender: SenderResponse,
    pub state: crate::models::platform_personal_binding::PersonalBindingState,
    pub conversation_id: Option<String>,
    pub consented_at: String,
    pub revoked_at: Option<String>,
}
#[derive(Serialize)]
pub struct ListResponse {
    pub bindings: Vec<OwnedBindingResponse>,
    pub next_before_id: Option<String>,
}
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(input): Query<ListInput>,
) -> AppResult<(HeaderMap, Json<ListResponse>)> {
    super::login_client_context::require_first_party_human(&auth)?;
    let cursor = input.before_id.map(|id| id.to_string());
    let mut rows =
        bindings::list_owned(&state.db, &auth.user_id.to_string(), cursor.as_deref()).await?;
    let more = rows.len() > 50;
    rows.truncate(50);
    let next_before_id = if more {
        rows.last().map(|r| r.id.clone())
    } else {
        None
    };
    let mut projected = Vec::with_capacity(rows.len());
    let mut displays = std::collections::HashMap::new();
    for row in rows {
        let key = (row.platform_config_id.clone(), row.manager_bot_id.clone());
        let display = if let Some(display) = displays.get(&key) {
            Clone::clone(display)
        } else {
            let display = manager(
                &state.db,
                &row.platform_config_id,
                &row.manager_bot_id,
                &row.entry,
                false,
            )
            .await?;
            displays.insert(key, display.clone());
            display
        };
        projected.push(OwnedBindingResponse {
            chat: chat(&row.entry)?,
            id: row.id,
            generation: row.generation,
            manager: display,
            sender: SenderResponse {
                id: crate::services::platform_entry_scope::actor_string(&row.sender),
                display_name: None,
            },
            state: row.state,
            conversation_id: row.conversation_id,
            consented_at: row.consented_at.to_rfc3339(),
            revoked_at: row.revoked_at.map(|date| date.to_rfc3339()),
        });
    }
    Ok((
        private_headers(),
        Json(ListResponse {
            bindings: projected,
            next_before_id,
        }),
    ))
}

#[derive(Serialize)]
pub struct RevokedResponse {
    pub revoked: bool,
    pub work_may_continue_in_app: bool,
}
pub async fn revoke(
    State(state): State<AppState>,
    auth: AuthUser,
    Extension(producers): Extension<
        Arc<crate::services::platform_manager_contract::PlatformManagerProducers>,
    >,
    Path(id): Path<uuid::Uuid>,
    Json(input): Json<RevokeInput>,
) -> AppResult<(HeaderMap, Json<RevokedResponse>)> {
    super::login_client_context::require_first_party_human(&auth)?;
    if input.generation <= 0 {
        return Err(AppError::ValidationError(
            "Invalid binding generation".into(),
        ));
    }
    bindings::revoke_owned(
        &state.db,
        &crate::services::audit_service::AuditActor::from_auth_user(&auth),
        state.audit_chain_hmac_key.as_slice(),
        &id.to_string(),
        input.generation,
        &bindings::LifecycleProducer {
            personal: producers.personal.clone(),
            public: producers.public.clone(),
        },
    )
    .await?;
    Ok((
        private_headers(),
        Json(RevokedResponse {
            revoked: true,
            work_may_continue_in_app: true,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::platform_personal_binding_service::tests::fixture;
    use mongodb::bson::{Bson, Document, doc};

    #[tokio::test]
    async fn legacy_person_can_review_but_explicit_null_org_and_unknown_fail_the_fence() {
        let (db, update, owner) = fixture("personal_legacy_human").await;
        let users = db.collection::<Document>(crate::models::user::COLLECTION_NAME);
        users
            .update_one(doc! {"_id":&owner}, doc! {"$unset":{"user_type":""}})
            .await
            .unwrap();
        let state = crate::test_utils::test_app_state(db.clone());
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        assert!(
            bindings::fence_person(&db, &mut session, &owner)
                .await
                .unwrap()
        );
        let issued = proofs::begin_in_session(
            &db,
            &mut session,
            state.auth_device_hmac_key.as_slice(),
            &update,
            false,
            "legacy-review",
        )
        .await
        .unwrap();
        session.commit_transaction().await.unwrap();
        let (_, Json(reviewed)) = review(
            State(state),
            crate::test_utils::test_auth_user(&owner),
            Json(IntentInput {
                intent_token: issued.token.unwrap(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(reviewed.account.id, owner);
        assert_eq!(reviewed.permission_mode, "nyxmode");
        for invalid in [
            Bson::Null,
            Bson::String("org".into()),
            Bson::String("unknown".into()),
        ] {
            users
                .update_one(doc! {"_id":&owner}, doc! {"$set":{"user_type":invalid}})
                .await
                .unwrap();
            session.start_transaction().await.unwrap();
            assert!(
                !bindings::fence_person(&db, &mut session, &owner)
                    .await
                    .unwrap()
            );
            session.abort_transaction().await.unwrap();
        }
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn consent_manager_uses_only_metadata_for_the_current_verified_observation() {
        let (db, update, _) = fixture("personal_manager_display").await;
        let credentials =
            db.collection::<Document>(crate::models::platform_credential::COLLECTION_NAME);
        credentials
            .update_one(
                doc! {"provider":"telegram-new"},
                doc! {"$set":{"fields.manager_username":"VerifiedManagerBot"}},
            )
            .await
            .unwrap();
        let metadata = manager(
            &db,
            &update.profile_id,
            &update.manager_bot_id,
            &update.entry,
            true,
        )
        .await
        .unwrap();
        assert_eq!(metadata.id, update.profile_id);
        assert_eq!(metadata.username.as_deref(), Some("VerifiedManagerBot"));
        assert_eq!(metadata.display_name, "@VerifiedManagerBot");
        credentials
            .update_one(
                doc! {"provider":"telegram-new"},
                doc! {"$set":{"fields.observation_id":"unrelated-observation"}},
            )
            .await
            .unwrap();
        assert!(
            manager(
                &db,
                &update.profile_id,
                &update.manager_bot_id,
                &update.entry,
                true
            )
            .await
            .is_err()
        );
        let historical = manager(
            &db,
            &update.profile_id,
            &update.manager_bot_id,
            &update.entry,
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            historical.display_name,
            format!("Telegram bot {}", update.manager_bot_id)
        );
        assert!(historical.username.is_none());
        credentials.update_one(doc! {"provider":"telegram-new"},doc! {"$set":{"fields.observation_id":&update.observation_id},"$unset":{"fields.manager_username":""}}).await.unwrap();
        db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
            .update_one(
                doc! {"platform_profile_id":&update.profile_id},
                doc! {"$set":{"platform_bot_username":"ChannelManagerBot"}},
            )
            .await
            .unwrap();
        assert_eq!(
            manager(
                &db,
                &update.profile_id,
                &update.manager_bot_id,
                &update.entry,
                true
            )
            .await
            .unwrap()
            .username
            .as_deref(),
            Some("ChannelManagerBot")
        );
        db.drop().await.unwrap();
    }
}
