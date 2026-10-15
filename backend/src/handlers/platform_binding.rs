use crate::services::{audit_service, platform_binding_service as bindings};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::platform_binding::BindingRequestState,
    mw::auth::AuthUser,
};
use axum::{
    Json,
    extract::{ConnectInfo, Path, State},
    http::{HeaderMap, header},
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeInput {
    pub code: Zeroizing<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionInput {
    pub code: Zeroizing<String>,
    pub matched_code: Zeroizing<String>,
    #[serde(default)]
    pub replace_existing: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevokeInput {
    pub generation: i64,
}

#[derive(Serialize)]
pub struct PreviewResponse {
    pub request_id: String,
    pub manager_bot_id: String,
    pub requesting_sender_id: String,
    pub state: BindingRequestState,
    pub expires_at: String,
    pub scope: &'static str,
}
fn projected(row: crate::models::platform_binding::PlatformBindingRequest) -> PreviewResponse {
    PreviewResponse {
        request_id: row.id,
        manager_bot_id: row.manager_bot_id,
        requesting_sender_id: row.telegram_sender_id,
        state: row.state,
        expires_at: row.expires_at.to_rfc3339(),
        scope: "Onboarding status and reviewed setup drafts only; no account credentials or general account access",
    }
}
fn headers() -> HeaderMap {
    HeaderMap::from_iter([
        (header::CACHE_CONTROL, "no-store".parse().unwrap()),
        (header::REFERRER_POLICY, "no-referrer".parse().unwrap()),
    ])
}
fn key(state: &AppState) -> &[u8] {
    state.auth_device_hmac_key.as_slice()
}

pub async fn preview(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    headers_in: HeaderMap,
    Json(body): Json<CodeInput>,
) -> AppResult<(HeaderMap, Json<PreviewResponse>)> {
    ingress_limit(&state, &headers_in, peer, "platform_binding_preview_ip", 30).await?;
    Ok((
        headers(),
        Json(projected(
            bindings::preview(&state.db, key(&state), &body.code).await?,
        )),
    ))
}

async fn ingress_limit(
    state: &AppState,
    headers: &HeaderMap,
    peer: std::net::SocketAddr,
    namespace: &str,
    cap: u32,
) -> AppResult<()> {
    let ip = super::login_client_context::resolve_client_context(headers, peer, state)?.ip;
    let hash = crate::services::auth_device_service::hmac_hex(
        key(state),
        format!("platform-binding-ip:{ip}").as_bytes(),
    );
    if !crate::mw::rate_limit::PerKeyRateLimiter::with_db(state.db.clone(), namespace, cap, 60)
        .check_shared(&hash)
        .await?
    {
        return Err(AppError::RateLimited);
    }
    Ok(())
}

async fn decide(
    state: &AppState,
    auth: &AuthUser,
    body: &DecisionInput,
    approve: bool,
) -> AppResult<(HeaderMap, Json<PreviewResponse>)> {
    super::login_client_context::require_first_party_human(auth)?;
    let actor = auth.user_id.to_string();
    if !crate::mw::rate_limit::PerKeyRateLimiter::with_db(
        state.db.clone(),
        "platform_binding_human_decision",
        20,
        60,
    )
    .check_shared(&actor)
    .await?
    {
        return Err(AppError::RateLimited);
    }
    let row = bindings::decide(
        &state.db,
        key(state),
        &actor,
        &body.code,
        &body.matched_code,
        approve,
        body.replace_existing,
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        auth,
        if approve {
            "platform_binding_reviewed"
        } else {
            "platform_binding_denied"
        },
        Some(
            serde_json::json!({"binding_request_id": row.id, "platform_config_id": row.platform_config_id}),
        ),
    );
    Ok((headers(), Json(projected(row))))
}
pub async fn approve(
    State(state): State<AppState>,
    auth: AuthUser,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    headers_in: HeaderMap,
    Json(body): Json<DecisionInput>,
) -> AppResult<(HeaderMap, Json<PreviewResponse>)> {
    ingress_limit(
        &state,
        &headers_in,
        peer,
        "platform_binding_decision_ip",
        30,
    )
    .await?;
    decide(&state, &auth, &body, true).await
}
pub async fn deny(
    State(state): State<AppState>,
    auth: AuthUser,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    headers_in: HeaderMap,
    Json(body): Json<DecisionInput>,
) -> AppResult<(HeaderMap, Json<PreviewResponse>)> {
    ingress_limit(
        &state,
        &headers_in,
        peer,
        "platform_binding_decision_ip",
        30,
    )
    .await?;
    decide(&state, &auth, &body, false).await
}
#[derive(Serialize)]
pub struct OwnedBindingResponse {
    pub id: String,
    pub generation: i64,
    pub platform_config_id: String,
    pub manager_bot_id: String,
    pub telegram_sender_id: String,
    pub expires_at: String,
}

pub async fn list_owned(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<(HeaderMap, Json<Vec<OwnedBindingResponse>>)> {
    super::login_client_context::require_first_party_human(&auth)?;
    let rows = bindings::list_owned(&state.db, &auth.user_id.to_string()).await?;
    let output = rows
        .into_iter()
        .map(|row| OwnedBindingResponse {
            id: row.id,
            generation: row.generation,
            platform_config_id: row.platform_config_id,
            manager_bot_id: row.manager_bot_id,
            telegram_sender_id: row.telegram_sender_id,
            expires_at: row.expires_at.to_rfc3339(),
        })
        .collect();
    Ok((
        HeaderMap::from_iter([(header::CACHE_CONTROL, "no-store".parse().unwrap())]),
        Json(output),
    ))
}

fn setup_status(state: crate::models::platform_setup::SetupState) -> &'static str {
    use crate::models::platform_setup::SetupState::*;
    match state {
        Draft => "Draft",
        AwaitingIdentity => "Account linking required",
        Collecting => "Collecting details",
        AwaitingApproval => "Waiting for your review",
        Provisioning => "Preparing your bot",
        AwaitingChildReview => "Review the created Telegram bot",
        ReadyForTest => "Ready for customer test",
        Active => "Active",
        FailedRetryable => "Needs recovery",
        Paused => "Paused",
        Cancelled => "Cancelled",
        Expired => "Expired",
    }
}

pub async fn revoke(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<uuid::Uuid>,
    Json(body): Json<RevokeInput>,
) -> AppResult<axum::http::StatusCode> {
    super::login_client_context::require_first_party_human(&auth)?;
    bindings::revoke_owned(
        &state.db,
        &auth.user_id.to_string(),
        &id.to_string(),
        body.generation,
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "platform_binding_revoked",
        Some(serde_json::json!({"binding_id": id.to_string()})),
    );
    Ok(axum::http::StatusCode::NO_CONTENT)
}

fn command_error_reply(error: &AppError) -> Option<&'static str> {
    match error {
        AppError::RateLimited | AppError::ConnectLinkRateLimited => {
            Some("Too many linking attempts. Wait before trying again.")
        }
        AppError::NotFound(_) | AppError::ValidationError(_) | AppError::Conflict(_) => Some(
            "This linking request is unavailable or expired. Send /link to start again, review the same code in your browser, then return here to confirm.",
        ),
        _ => None,
    }
}

/// Called only after the existing manager receiver verified its webhook secret
/// and classified native creation/control updates. No bot token enters a turn.
pub(crate) async fn handle_command(
    state: &AppState,
    bot: &crate::models::channel_bot::ChannelBot,
    update: &serde_json::Value,
    service: &crate::services::telegram_new_service::TelegramNewService<'_>,
) -> AppResult<bool> {
    let profile = state
        .db
        .collection::<crate::models::platform_bot_config::PlatformBotConfig>(
            crate::models::platform_bot_config::COLLECTION_NAME,
        )
        .find_one(bson::doc! {"manager_bot_id": &bot.platform_bot_id})
        .await?;
    let Some(profile) = profile else {
        return Ok(false);
    };
    let message = &update["message"];
    let text = message["text"].as_str().unwrap_or("");
    let Some(command) = text.split_whitespace().next() else {
        return Ok(false);
    };
    let base = command.split('@').next().unwrap_or("");
    if !matches!(base, "/link" | "/confirm" | "/unlink" | "/setups") {
        return Ok(false);
    }
    let (_, username, values) = service.manager().await?;
    if command
        .split_once('@')
        .is_some_and(|(_, name)| !name.eq_ignore_ascii_case(&username))
    {
        return Ok(true);
    }
    let sender = message["from"]["id"].as_u64();
    let chat = message["chat"]["id"].as_u64();
    let private = message["chat"]["type"] == "private"
        && sender.is_some()
        && sender == chat
        && message["from"]["is_bot"] == false
        && message.get("forward_origin").is_none();
    let target = message["chat"]["id"]
        .as_i64()
        .ok_or_else(|| AppError::ValidationError("Missing Telegram chat".into()))?;
    let mut reply_binding = None;
    let reply = if !private {
        "Open the manager's private chat to review account linking. Group chats cannot link or show client setups.".to_owned()
    } else if !profile.enabled || profile.deleted_at.is_some() {
        "Platform onboarding is paused. Native bot creation remains available; use NyxID's website to manage your own setups.".to_owned()
    } else {
        let sender = sender.unwrap().to_string();
        let chat = chat.unwrap().to_string();
        let prepared: AppResult<String> = async { Ok(match base {
            "/link" => {
                let (_, code) = bindings::begin(
                    &state.db,
                    key(state),
                    &profile,
                    &sender,
                    &chat,
                    update["update_id"].as_i64().ok_or_else(|| {
                        AppError::ValidationError("Missing Telegram update identity".into())
                    })?,
                )
                .await?;
                let mut url = reqwest::Url::parse(&state.config.frontend_url)
                    .map_err(|_| AppError::Internal("Frontend origin is invalid".into()))?;
                url.set_path("/onboarding/link");
                url.set_query(None);
                url.set_fragment(None);
                url.query_pairs_mut().append_pair("code", &code);
                let code = code.as_str();
                format!(
                    "Review your NyxID account at {url}\nMatch code {code} with this chat. Linking grants onboarding progress only. After browser approval, return here and send /confirm {code}. Never enter a password or bot token here."
                )
            }
            "/confirm" => {
                let code = text[command.len()..].trim();
                bindings::confirm_from_chat(&state.db, key(state), &profile, &sender, &chat, code)
                    .await?;
                "Account linking confirmed. Use your own NyxBot or trusted setup page for approvals. /setups shows onboarding progress; /unlink revokes this chat binding.".into()
            }
            "/unlink" => {
                bindings::unlink_sender(&state.db, &profile, &sender, &chat).await?;
                "This chat is unlinked. Existing setups remain owned by their original NyxID account.".into()
            }
            "/setups" => {
                if let Some(binding) =
                    bindings::live_for_sender(&state.db, &profile, &sender, &chat).await?
                {
                    reply_binding = Some(binding.clone());
                    let rows = crate::services::platform_setup_service::list_owned(
                        &state.db,
                        &binding.user_id,
                    )
                    .await?;
                    let mut reply = "Your onboarding setups:".to_owned();
                    for row in rows.into_iter().filter(|row| {
                        row.platform_config_id.as_deref() == Some(profile.id.as_str())
                            && row.binding_id.as_deref() == Some(binding.id.as_str())
                            && row.binding_generation == Some(binding.generation)
                    }) {
                        reply.push_str(&format!(
                            "\n{} — {}\n{}/onboarding/setups/{}",
                            row.id,
                            setup_status(row.state),
                            state.config.frontend_url.trim_end_matches('/'),
                            row.id
                        ));
                    }
                    reply
                } else {
                    "Link your own account first with /link. No account credentials are delivered to this bot.".into()
                }
            }
            _ => unreachable!(),
        }) }.await;
        match prepared {
            Ok(reply) => reply,
            Err(error) => {
                let Some(reply) = command_error_reply(&error) else {
                    return Err(error);
                };
                reply_binding = None;
                reply.into()
            }
        }
    };
    // Bound all deterministic command replies, including rejected guesses and
    // replayed updates; a provider failure is still acknowledged by the receiver.
    if !crate::mw::rate_limit::PerKeyRateLimiter::with_db(
        state.db.clone(),
        "platform_binding_command_reply",
        10,
        60,
    )
    .check_shared(&format!("{}:{target}", profile.id))
    .await?
    {
        return Ok(true);
    }
    let token = values
        .get(crate::services::channel_adapters::telegram_new::MANAGER_TOKEN)
        .ok_or_else(|| AppError::Internal("Manager credentials unavailable".into()))?;
    // Recheck before provider effects; a paused generation cannot send a stale
    // account-status response prepared by the previous profile.
    let live = crate::services::platform_bot_config_service::get(&state.db, &profile.id).await?;
    if live.generation != profile.generation {
        return Ok(true);
    }
    if profile.enabled && profile.deleted_at.is_none() {
        bindings::admit_reply(&state.db, &profile, reply_binding.as_ref()).await?;
    }
    if let Err(error) = service
        .api
        .call(
            token,
            "sendMessage",
            serde_json::json!({"chat_id": target, "text": reply, "protect_content": true}),
        )
        .await
    {
        tracing::warn!(
            error_code = error.error_code(),
            "Platform command notice delivery failed"
        );
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn platform_onboarding_command_errors_are_fixed_and_hide_internal_prose() {
        for error in [
            AppError::NotFound("private account details".into()),
            AppError::ValidationError("private code details".into()),
            AppError::Conflict("private account binding".into()),
        ] {
            let reply = command_error_reply(&error).unwrap();
            assert!(reply.contains("/link"));
            assert!(!reply.contains("private"));
        }
        assert!(
            command_error_reply(&AppError::RateLimited)
                .unwrap()
                .contains("Wait")
        );
        assert!(
            command_error_reply(&AppError::Internal("secret provider response".into())).is_none()
        );
    }
}
