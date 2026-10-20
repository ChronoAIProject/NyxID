use super::{channel_bots, channel_managed as managed, telegram_new};
use crate::models::channel_connect_link::{ChannelConnectLink, LinkStatus};
use crate::services::{channel_connect_link_service as links, channel_managed};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::AuthUser,
};
use axum::{
    Json,
    extract::{ConnectInfo, Path, State},
    http::{HeaderMap, StatusCode, header},
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateRequest {
    pub platform: String,
    pub label: String,
    pub target_org_id: Option<String>,
    pub requested_by: Option<String>,
    pub callback_url: Option<String>,
    pub webhook_url: Option<String>,
    pub expires_in: Option<i64>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct CreateResponse {
    pub id: String,
    pub connect_url: String,
    pub expires_at: String,
    pub webhook_signing_secret: Option<String>,
    pub webhook_signing_key_id: Option<String>,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct TokenRequest {
    #[schema(value_type = String)]
    pub token: Zeroizing<String>,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct CompleteRequest {
    #[schema(value_type = String)]
    pub token: Zeroizing<String>,
    #[serde(flatten)]
    pub input: serde_json::Map<String, serde_json::Value>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct StatusResponse {
    pub id: String,
    pub status: &'static str,
    pub platform: String,
    pub label: String,
    pub owner_id: Option<String>,
    pub requested_by: Option<String>,
    pub expires_at: String,
    pub bot_id: Option<String>,
    pub connection_id: Option<String>,
    pub telegram_request_id: Option<String>,
    pub telegram_requires_original_actor: bool,
    pub callback_url: Option<String>,
    pub last_error: Option<String>,
    pub delivery_status: Option<String>,
}

fn headers() -> HeaderMap {
    HeaderMap::from_iter([
        (header::CACHE_CONTROL, "no-store".parse().unwrap()),
        (header::REFERRER_POLICY, "no-referrer".parse().unwrap()),
    ])
}

fn status(link: ChannelConnectLink, private: bool) -> AppResult<StatusResponse> {
    let callback_url = if private {
        links::callback_url(&link)?
    } else {
        None
    };
    Ok(StatusResponse {
        id: link.id,
        status: links::status_name(link.status),
        platform: link.platform,
        label: link.label,
        owner_id: private.then_some(link.user_id),
        requested_by: link.requested_by,
        expires_at: link.expires_at.to_rfc3339(),
        bot_id: if private { link.bot_id } else { None },
        connection_id: if private { link.connection_id } else { None },
        telegram_request_id: if private {
            link.telegram_request_id
        } else {
            None
        },
        telegram_requires_original_actor: false,
        callback_url,
        last_error: if private { link.last_error } else { None },
        delivery_status: if private { link.delivery_status } else { None },
    })
}

async fn limit(state: &AppState, key: &str) -> AppResult<()> {
    if !crate::mw::rate_limit::PerKeyRateLimiter::with_db(
        state.db.clone(),
        "channel_connect_links",
        30,
        60,
    )
    .check_shared(key)
    .await?
    {
        return Err(AppError::RateLimited);
    }
    Ok(())
}

fn dispatch(state: &AppState, id: String) {
    let state = state.clone();
    tokio::spawn(async move {
        if let Err(error) = links::dispatch(
            &state.db,
            &state.encryption_keys,
            &state.developer_webhook_dispatcher,
            &id,
        )
        .await
        {
            tracing::warn!(channel_connect_link_id = %id, %error, "Channel connect webhook remains queued");
        }
    });
}

#[utoipa::path(post, path = "/api/v1/channel-connect-links", request_body = CreateRequest, responses((status = 200, description = "Channel connection request result", body = CreateResponse)), tag = "Channel Connect Links") ]
pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateRequest>,
) -> AppResult<(HeaderMap, Json<CreateResponse>)> {
    let actor = auth.user_id.to_string();
    limit(&state, auth.api_key_id.as_deref().unwrap_or(&actor)).await?;
    let adapter = channel_bots::resolve_adapter(&body.platform, &state.token_exchange_cache)?;
    if !adapter.registration().enabled {
        return Err(AppError::ValidationError(
            "Channel registration is disabled for this platform".into(),
        ));
    }
    let owner =
        channel_bots::resolve_create_owner(&state, &actor, body.target_org_id.as_deref()).await?;
    let created = links::create(
        &state.db,
        &state.encryption_keys,
        links::CreateInput {
            owner,
            actor,
            platform: adapter.platform_id().into(),
            label: body.label,
            requested_by: auth.api_key_name.clone().or(body.requested_by),
            app_id: auth.oauth_client_id.clone(),
            callback_url: body.callback_url,
            webhook_url: body.webhook_url,
            expires_in: body.expires_in,
        },
    )
    .await?;
    let connect_url = format!(
        "{}/connect/bot/{}",
        state.config.frontend_url.trim_end_matches('/'),
        created.token.as_str()
    );
    crate::services::audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "channel_connect_link_created",
        Some(
            serde_json::json!({"channel_connect_link_id": created.link.id, "platform": created.link.platform}),
        ),
    );
    Ok((
        headers(),
        Json(CreateResponse {
            id: created.link.id,
            connect_url,
            expires_at: created.link.expires_at.to_rfc3339(),
            webhook_signing_secret: created.signing_secret.map(|s| s.to_string()),
            webhook_signing_key_id: created.signing_key_id,
        }),
    ))
}

#[utoipa::path(post, path = "/api/v1/channel-connect-links/preview", request_body = TokenRequest, responses((status = 200, description = "Channel connection request result", body = StatusResponse)), tag = "Channel Connect Links") ]
pub async fn preview(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<std::net::SocketAddr>,
    request_headers: HeaderMap,
    Json(body): Json<TokenRequest>,
) -> AppResult<(HeaderMap, Json<StatusResponse>)> {
    let ip = crate::mw::rate_limit::resolve_client_ip_for_rate_limit(
        &request_headers,
        Some(addr),
        &state.config.trusted_proxy_ips,
    )
    .ok_or_else(|| AppError::ValidationError("Unable to resolve client address".into()))?;
    limit(&state, &format!("preview:{ip}")).await?;
    // Preview reveals no owner, bot, provider handles, or notification destination.
    let link = links::by_token(&state.db, &body.token).await?;
    Ok((headers(), Json(status(link, false)?)))
}

#[utoipa::path(get, path = "/api/v1/channel-connect-links/{id}", params(("id" = String, Path, description = "Channel connection request ID")), responses((status = 200, description = "Channel connection request result", body = StatusResponse)), tag = "Channel Connect Links") ]
pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<(HeaderMap, Json<StatusResponse>)> {
    let link = links::get(&state.db, &id).await?;
    links::authorize(&state.db, &auth.user_id.to_string(), &link).await?;
    links::reconcile(&state.db, &link).await?;
    dispatch(&state, id.clone());
    let link = links::get(&state.db, &id).await?;
    let requires_actor =
        links::telegram_requires_original_actor(&state.db, &auth.user_id.to_string(), &link)
            .await?;
    let mut response = status(link, true)?;
    response.telegram_requires_original_actor = requires_actor;
    Ok((headers(), Json(response)))
}

#[utoipa::path(post, path = "/api/v1/channel-connect-links/{id}/cancel", params(("id" = String, Path, description = "Channel connection request ID")), responses((status = 200, description = "Channel connection request result", body = StatusResponse)), tag = "Channel Connect Links") ]
pub async fn cancel(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<(HeaderMap, Json<StatusResponse>)> {
    let link = links::get(&state.db, &id).await?;
    links::authorize(&state.db, &auth.user_id.to_string(), &link).await?;
    let link = links::cancel(&state.db, &link).await?;
    dispatch(&state, id);
    Ok((headers(), Json(status(link, true)?)))
}

#[utoipa::path(post, path = "/api/v1/channel-connect-links/decline", request_body = TokenRequest, responses((status = 200, description = "Channel connection request result", body = StatusResponse)), tag = "Channel Connect Links") ]
pub async fn decline(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<TokenRequest>,
) -> AppResult<(HeaderMap, Json<StatusResponse>)> {
    let link = links::by_token(&state.db, &body.token).await?;
    cancel(State(state), auth, Path(link.id)).await
}

fn bind_input(
    input: &mut serde_json::Map<String, serde_json::Value>,
    claim: &links::Claim,
    actor: &str,
) -> AppResult<()> {
    for (key, expected) in [
        ("platform", claim.link.platform.as_str()),
        ("label", claim.link.label.as_str()),
    ] {
        if input.get(key).is_some_and(|v| v.as_str() != Some(expected)) {
            return Err(AppError::ValidationError(format!(
                "{key} is fixed by this connect link"
            )));
        }
    }
    if let Some(owner) = input
        .get("target_org_id")
        .and_then(serde_json::Value::as_str)
        && owner != claim.link.user_id
    {
        return Err(AppError::ValidationError(
            "The owner is fixed by this connect link".into(),
        ));
    }
    input.insert("label".into(), claim.link.label.clone().into());
    if claim.link.user_id != actor {
        input.insert("target_org_id".into(), claim.link.user_id.clone().into());
    } else {
        input.remove("target_org_id");
    }
    Ok(())
}

async fn finish<T>(state: &AppState, claim: &links::Claim, result: AppResult<T>) -> AppResult<T> {
    let incomplete = if result.is_ok() {
        match links::complete(&state.db, &claim.link.id).await {
            Ok(()) => false,
            Err(error) => {
                tracing::warn!(channel_connect_link_id = %claim.link.id, %error, "Channel setup result saved; completion will reconcile");
                true
            }
        }
    } else {
        true
    };
    // The successful response may contain a secret that cannot be read again.
    if incomplete && let Err(error) = links::release(&state.db, claim, true).await {
        tracing::warn!(channel_connect_link_id = %claim.link.id, %error, "Channel setup claim awaits expiry");
    }
    dispatch(state, claim.link.id.clone());
    result
}

#[utoipa::path(post, path = "/api/v1/channel-connect-links/complete", request_body = CompleteRequest, responses((status = 201, description = "Channel connection request result", body = serde_json::Value)), tag = "Channel Connect Links") ]
pub async fn complete(
    State(state): State<AppState>,
    auth: AuthUser,
    tele: crate::telemetry::TelemetryContext,
    Json(mut body): Json<CompleteRequest>,
) -> AppResult<(StatusCode, Json<channel_bots::CreateChannelBotResponse>)> {
    let actor = auth.user_id.to_string();
    limit(&state, &actor).await?;
    let claim = links::claim(&state.db, &actor, &body.token, "manual").await?;
    tokio::spawn(async move {
        let result = links::with_claim(&state.db, &claim, async {
            if claim.link.bot_id.is_some() {
                return Err(AppError::Conflict(
                    "This bot already exists; use Retry setup to verify it".into(),
                ));
            }
            bind_input(&mut body.input, &claim, &actor)?;
            body.input
                .insert("platform".into(), claim.link.platform.clone().into());
            let input = serde_json::from_value(serde_json::Value::Object(body.input))
                .map_err(|_| AppError::ValidationError("Invalid bot setup fields".into()))?;
            let adapter =
                channel_bots::resolve_adapter(&claim.link.platform, &state.token_exchange_cache)?;
            channel_bots::create_bot_with_link(
                &state,
                auth,
                tele,
                input,
                adapter.as_ref(),
                &crate::services::telegram_new_api::TelegramApi::new(&state.http_client),
                Some(&claim),
            )
            .await
        })
        .await;
        finish(&state, &claim, result).await
    })
    .await
    .map_err(|_| AppError::Internal("Channel setup was interrupted".into()))?
}

#[utoipa::path(post, path = "/api/v1/channel-connect-links/managed/start", request_body = TokenRequest, responses((status = 200, description = "Channel connection request result", body = serde_json::Value)), tag = "Channel Connect Links") ]
pub async fn managed_start(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<TokenRequest>,
) -> AppResult<(HeaderMap, Json<managed::StartResponse>)> {
    let actor = auth.user_id.to_string();
    limit(&state, &actor).await?;
    managed::limit(&state, &auth).await?;
    let claim = links::claim(&state.db, &actor, &body.token, "managed").await?;
    tokio::spawn(async move {
        let result = links::with_claim(&state.db, &claim, async {
            if claim.link.bot_id.is_some() {
                return Err(AppError::Conflict("Resume the existing bot setup".into()));
            }
            let adapter =
                channel_bots::resolve_adapter(&claim.link.platform, &state.token_exchange_cache)?;
            let started = crate::services::channel_credentials::start_connection(
                &state.db,
                &state.encryption_keys,
                &state.config.base_url,
                adapter.as_ref(),
                &actor,
                &claim.link.user_id,
                &claim.link.label,
            )
            .await?;
            links::pin_connection(&state.db, &claim, &started.connection_id).await?;
            Ok((
                headers(),
                Json(managed::StartResponse {
                    connection_id: started.connection_id,
                    authorization_url: started.authorization_url,
                    attempt_nonce: started.attempt_nonce,
                }),
            ))
        })
        .await;
        links::release(&state.db, &claim, result.is_err()).await?;
        result
    })
    .await
    .map_err(|_| AppError::Internal("Channel setup was interrupted".into()))?
}

#[utoipa::path(post, path = "/api/v1/channel-connect-links/managed/complete", request_body = CompleteRequest, responses((status = 201, description = "Channel connection request result", body = serde_json::Value)), tag = "Channel Connect Links") ]
pub async fn managed_complete(
    State(state): State<AppState>,
    auth: AuthUser,
    request_headers: HeaderMap,
    Json(mut body): Json<CompleteRequest>,
) -> AppResult<axum::response::Response> {
    use axum::response::{IntoResponse, Sse, sse::Event};
    let actor = auth.user_id.to_string();
    limit(&state, &actor).await?;
    managed::limit(&state, &auth).await?;
    let claim = links::claim(&state.db, &actor, &body.token, "managed").await?;
    let streaming = request_headers
        .get(header::ACCEPT)
        .is_some_and(|v| v == "text/event-stream");
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    let progress = channel_managed::ManagedProgress(streaming.then(|| sender.clone()));
    // Complete server-side even if the browser leaves during the provider operation.
    let task = tokio::spawn(async move {
        let result = links::with_claim(&state.db, &claim, async {
            if claim.link.bot_id.is_some() {
                return Err(AppError::Conflict(
                    "This bot already exists; use Retry setup".into(),
                ));
            }
            bind_input(&mut body.input, &claim, &actor)?;
            body.input.remove("platform");
            if let Some(id) = body
                .input
                .get("connection_id")
                .and_then(serde_json::Value::as_str)
                && claim.link.connection_id.as_deref() != Some(id)
            {
                return Err(AppError::ValidationError(
                    "OAuth connection does not belong to this link".into(),
                ));
            }
            let input = serde_json::from_value(serde_json::Value::Object(body.input))
                .map_err(|_| AppError::ValidationError("Invalid managed setup fields".into()))?;
            managed::complete_with_link(
                &state,
                &auth,
                &claim.link.platform,
                input,
                &progress,
                Some(&claim),
            )
            .await
        })
        .await;
        finish(&state, &claim, result).await
    });
    if streaming {
        tokio::spawn(async move {
            let result = task.await.unwrap_or_else(|_| {
                Err(AppError::Internal("Channel setup was interrupted".into()))
            });
            let event = match result {
                Ok((_, Json(result))) => serde_json::json!({ "result": result }),
                Err(error) => serde_json::json!(error.response_body()),
            };
            let _ = sender.send(event);
        });
        let stream = futures::stream::unfold(receiver, |mut receiver| async move {
            receiver.recv().await.map(|value| {
                (
                    Ok::<_, std::convert::Infallible>(Event::default().data(value.to_string())),
                    receiver,
                )
            })
        });
        let mut response_headers = headers();
        response_headers.insert(
            header::HeaderName::from_static("x-accel-buffering"),
            "no".parse().unwrap(),
        );
        return Ok((
            response_headers,
            Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default()),
        )
            .into_response());
    }
    Ok((
        headers(),
        task.await
            .map_err(|_| AppError::Internal("Channel setup was interrupted".into()))??,
    )
        .into_response())
}

#[utoipa::path(post, path = "/api/v1/channel-connect-links/telegram/start", request_body = TokenRequest, responses((status = 200, description = "Channel connection request result", body = serde_json::Value)), tag = "Channel Connect Links") ]
pub async fn telegram_start(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<TokenRequest>,
) -> AppResult<(HeaderMap, Json<telegram_new::LaunchResponse>)> {
    let actor = auth.user_id.to_string();
    limit(&state, &actor).await?;
    let claim = links::claim(&state.db, &actor, &body.token, "telegram").await?;
    tokio::spawn(async move {
        let result = links::with_claim(&state.db, &claim, async {
            if claim.link.platform != "telegram-new" {
                return Err(AppError::ValidationError(
                    "This link is not for Telegram creation".into(),
                ));
            }
            let service = telegram_new::service(&state);
            let existing = state
                .db
                .collection::<crate::models::telegram_bot_request::TelegramBotRequest>(
                    crate::models::telegram_bot_request::COLLECTION_NAME,
                )
                .find_one(bson::doc! {"_id": &claim.link.id, "owner_user_id": &claim.link.user_id})
                .await?;
            let (request, launch_url) = if let Some(request) = existing {
                if request.actor_user_id != actor {
                    return Err(AppError::Conflict(
                        "Continue Telegram setup using the NyxID account that started it".into(),
                    ));
                }
                let url = service.launch(&actor, &request.id).await?;
                (request, url)
            } else {
                service
                    .begin_linked(&actor, &claim.link.user_id, &claim.link.label, &claim)
                    .await?
            };
            Ok((
                headers(),
                Json(telegram_new::LaunchResponse {
                    request: request.into(),
                    launch_url,
                }),
            ))
        })
        .await;
        links::release(&state.db, &claim, result.is_err()).await?;
        result
    })
    .await
    .map_err(|_| AppError::Internal("Channel setup was interrupted".into()))?
}

#[utoipa::path(post, path = "/api/v1/channel-connect-links/retry", request_body = TokenRequest, responses((status = 200, description = "Channel connection request result", body = StatusResponse)), tag = "Channel Connect Links") ]
pub async fn retry(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<TokenRequest>,
) -> AppResult<(HeaderMap, Json<StatusResponse>)> {
    let actor = auth.user_id.to_string();
    limit(&state, &actor).await?;
    let link = links::by_token(&state.db, &body.token).await?;
    links::authorize(&state.db, &actor, &link).await?;
    if link.status != LinkStatus::Pending {
        return Ok((headers(), Json(status(link, true)?)));
    }
    let claim = links::claim(
        &state.db,
        &actor,
        &body.token,
        link.flow.as_deref().unwrap_or("manual"),
    )
    .await?;
    tokio::spawn(async move {
        let result = links::with_claim(&state.db, &claim, async {
            let id = claim
                .link
                .bot_id
                .as_deref()
                .ok_or_else(|| AppError::Conflict("No bot has been created yet".into()))?;
            let bot = crate::services::channel_bot_service::get_bot(&state.db, id).await?;
            if bot.managed_setup.is_some() {
                let _ = managed::repair(State(state.clone()), auth, Path(id.into())).await?;
            } else {
                let _ =
                    channel_bots::verify_bot(State(state.clone()), auth, Path(id.into())).await?;
            }
            Ok(())
        })
        .await;
        finish(&state, &claim, result).await?;
        Ok((
            headers(),
            Json(status(links::get(&state.db, &claim.link.id).await?, true)?),
        ))
    })
    .await
    .map_err(|_| AppError::Internal("Channel setup was interrupted".into()))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{connect_transaction_test_database, test_app_state, test_auth_user};

    #[tokio::test]
    async fn channel_link_successful_secret_response_survives_pending_finalization() {
        let db = connect_transaction_test_database("channel_link_finish_secret").await;
        let state = test_app_state(db);
        let actor = uuid::Uuid::new_v4().to_string();
        let created = links::create(
            &state.db,
            &state.encryption_keys,
            links::CreateInput {
                actor: actor.clone(),
                owner: actor.clone(),
                platform: "discord".into(),
                label: "Support".into(),
                requested_by: None,
                app_id: None,
                callback_url: None,
                webhook_url: None,
                expires_in: None,
            },
        )
        .await
        .unwrap();
        let claim = links::claim(&state.db, &actor, &created.token, "manual")
            .await
            .unwrap();
        // No bound bot makes finalization fail while provider success still has to be returned.
        let result = finish(
            &state,
            &claim,
            Ok(serde_json::json!({"webhook_secret": "only-response-copy"})),
        )
        .await
        .unwrap();
        assert_eq!(result["webhook_secret"], "only-response-copy");
        let link = links::get(&state.db, &created.link.id).await.unwrap();
        assert_eq!(link.status, LinkStatus::Pending);
        assert!(link.claim_id.is_none());
        assert!(link.event_id.is_none());
        assert_eq!(link.last_error.as_deref(), Some("setup_incomplete"));
    }

    #[tokio::test]
    async fn channel_link_managed_stream_reports_errors_and_releases_claim() {
        use axum::body::to_bytes;
        let db = connect_transaction_test_database("channel_link_managed_stream").await;
        let state = test_app_state(db);
        let actor = uuid::Uuid::new_v4().to_string();
        let created = links::create(
            &state.db,
            &state.encryption_keys,
            links::CreateInput {
                actor: actor.clone(),
                owner: actor.clone(),
                platform: "discord".into(),
                label: "Support".into(),
                requested_by: None,
                app_id: None,
                callback_url: None,
                webhook_url: None,
                expires_in: None,
            },
        )
        .await
        .unwrap();
        let response = managed_complete(
            State(state.clone()),
            test_auth_user(&actor),
            HeaderMap::from_iter([(header::ACCEPT, "text/event-stream".parse().unwrap())]),
            Json(CompleteRequest {
                token: created.token.clone(),
                input: serde_json::json!({"label": "Override"})
                    .as_object()
                    .unwrap()
                    .clone(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/event-stream"
        );
        assert_eq!(response.headers()["x-accel-buffering"], "no");
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        assert!(
            String::from_utf8(body.to_vec())
                .unwrap()
                .contains("label is fixed")
        );
        assert!(
            links::get(&state.db, &created.link.id)
                .await
                .unwrap()
                .claim_id
                .is_none()
        );
        // Both old and linked managed endpoints share the original five-attempt bucket.
        for _ in 0..4 {
            managed::limit(&state, &test_auth_user(&actor))
                .await
                .unwrap();
        }
        assert!(matches!(
            managed::limit(&state, &test_auth_user(&actor)).await,
            Err(AppError::RateLimited)
        ));
    }

    #[tokio::test]
    async fn channel_link_preview_hides_owner_and_destinations_and_binding_is_immutable() {
        let db = connect_transaction_test_database("channel_link_handler").await;
        let state = test_app_state(db);
        let actor = uuid::Uuid::new_v4().to_string();
        let (_, Json(created)) = create(
            State(state.clone()),
            test_auth_user(&actor),
            Json(CreateRequest {
                platform: "discord".into(),
                label: "Support".into(),
                target_org_id: None,
                requested_by: Some("App".into()),
                callback_url: Some("https://example.com/return".into()),
                webhook_url: None,
                expires_in: None,
            }),
        )
        .await
        .unwrap();
        let token = created.connect_url.rsplit('/').next().unwrap();
        let (headers, Json(preview)) = preview(
            State(state.clone()),
            ConnectInfo("127.0.0.1:4000".parse().unwrap()),
            HeaderMap::new(),
            Json(TokenRequest {
                token: Zeroizing::new(token.into()),
            }),
        )
        .await
        .unwrap();
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        assert_eq!(headers[header::REFERRER_POLICY], "no-referrer");
        assert!(preview.owner_id.is_none());
        assert!(preview.callback_url.is_none());
        assert!(preview.connection_id.is_none());
        assert!(preview.bot_id.is_none());
        let claim = links::claim(&state.db, &actor, token, "manual")
            .await
            .unwrap();
        for input in [
            serde_json::json!({"label": "Override"}),
            serde_json::json!({"platform": "telegram"}),
            serde_json::json!({"target_org_id": "another-owner"}),
        ] {
            assert!(bind_input(&mut input.as_object().unwrap().clone(), &claim, &actor).is_err());
        }
        let mut empty = serde_json::Map::new();
        bind_input(&mut empty, &claim, &actor).unwrap();
        assert_eq!(empty["label"], "Support");
        assert!(empty.get("target_org_id").is_none());
        assert!(
            get(
                State(state),
                test_auth_user(&uuid::Uuid::new_v4().to_string()),
                Path(created.id)
            )
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn channel_link_completion_routes_reject_agent_keys_like_connector_completion() {
        use axum::{body::Body, http::Request};
        use tower::ServiceExt;
        let db = connect_transaction_test_database("channel_link_routes").await;
        let state = test_app_state(db);
        let (_, private) = crate::routes::build_router_with_state(state.clone());
        let app = private.with_state(state);
        for path in [
            "/channel-connect-links/complete",
            "/channel-connect-links/decline",
            "/channel-connect-links/retry",
            "/channel-connect-links/managed/start",
            "/channel-connect-links/managed/complete",
            "/channel-connect-links/telegram/start",
            "/connect-links/complete",
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(format!("/api/v1{path}"))
                        .header("x-api-key", "nyx_test_agent_key")
                        .header("content-type", "application/json")
                        .body(Body::from(r#"{"token":"nyx_bcl_test"}"#))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
        }
    }
}
