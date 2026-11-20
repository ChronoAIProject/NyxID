use super::{
    auth_device::{AuthDevicePreviewResponse, preview_response},
    login_client_context::{auth_device_attribution, resolve_client_context},
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::{AuthUser, require_first_party_human},
    services::staging_qr::{self, Action, Binding, Outcome, Peer, Request},
};
use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum Reply {
    Preview {
        preview: Box<AuthDevicePreviewResponse>,
        binding: Binding,
    },
    Decision {
        ok: bool,
    },
}

#[derive(Deserialize)]
pub struct SignedRequest {
    proof: String,
}

fn unavailable() -> AppError {
    AppError::Internal("Staging QR service unavailable".into())
}

pub async fn forward(
    State(state): State<AppState>,
    user: AuthUser,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<Request>,
) -> AppResult<Json<Reply>> {
    require_first_party_human(&user)?;
    let Peer::Sender { client, endpoint } = &state.staging_qr else {
        return Err(staging_qr::forbidden());
    };
    let viewer = resolve_client_context(&headers, addr, &state)?;
    let action = request.action;
    let reviewed = request.binding.clone();
    let limiter = if action == Action::Preview {
        &state.auth_device_preview_limiter
    } else {
        &state.auth_device_approve_limiter
    };
    if !limiter.check_shared(viewer.ip).await?
        || (action != Action::Preview
            && !state
                .auth_device_approve_per_user_limiter
                .check_shared(&format!("user:{}", user.user_id))
                .await?)
    {
        return Err(AppError::AuthDeviceCodeRateLimited);
    }
    let proof = staging_qr::sign(
        &state.db,
        &state.jwt_keys,
        user.user_id,
        user.derived_mfa_authority(),
        viewer.ip,
        auth_device_attribution(viewer.attribution),
        request,
    )
    .await?;
    #[derive(Serialize)]
    struct Envelope<'a> {
        proof: &'a str,
    }
    let mut response = client
        .post(endpoint)
        .json(&Envelope {
            proof: proof.as_str(),
        })
        .send()
        .await
        .map_err(|_| unavailable())?;
    let status = response.status();
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| unavailable())? {
        if bytes.len() + chunk.len() > 16 * 1024 {
            return Err(unavailable());
        }
        bytes.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        let code = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|v| v.get("error_code")?.as_u64());
        return Err(match code {
            Some(11200 | 11207) => AppError::AuthDeviceUserCodeInvalid,
            Some(11201) => AppError::AuthDeviceCodeExpired,
            Some(11204) => AppError::AuthDeviceCodeDenied,
            Some(11205) => AppError::AuthDeviceCodeAlreadyDelivered,
            Some(11206) => AppError::AuthDeviceCodeRateLimited,
            Some(2003) => AppError::MfaStepUpRequired,
            _ if status == reqwest::StatusCode::FORBIDDEN => staging_qr::forbidden(),
            _ => unavailable(),
        });
    }
    let reply: Reply = serde_json::from_slice(&bytes).map_err(|_| unavailable())?;
    match (&reply, action) {
        (Reply::Preview { binding, .. }, Action::Preview)
            if binding.production_user == user.user_id
                && reviewed.as_ref().is_none_or(|expected| expected == binding) => {}
        (Reply::Decision { ok: true }, Action::Approve | Action::Deny) => {
            crate::services::audit_service::log_async(
                state.db.clone(), Some(user.user_id.to_string()), "staging_qr_login_decision".into(),
                reviewed.map(|b| serde_json::json!({"action":action, "request_id":b.request_id, "staging_user_id":b.staging_user})),
                Some(viewer.ip.to_string()), user.user_agent.clone(), None, None,
            );
        }
        _ => return Err(unavailable()),
    }
    Ok(Json(reply))
}

pub async fn receive(
    State(state): State<AppState>,
    Json(body): Json<SignedRequest>,
) -> AppResult<Json<Reply>> {
    let Peer::Receiver(key) = &state.staging_qr else {
        return Err(staging_qr::forbidden());
    };
    let proof = staging_qr::verify(key, &body.proof)?;
    let outcome = staging_qr::apply(
        &state.db,
        &state.config,
        &state.jwt_keys,
        &state.encryption_keys,
        state.auth_device_hmac_key.as_slice(),
        proof,
    )
    .await?;
    Ok(Json(match outcome {
        Outcome::Preview(preview, binding) => Reply::Preview {
            preview: Box::new(preview_response(*preview)),
            binding,
        },
        Outcome::Decision => Reply::Decision { ok: true },
    }))
}
