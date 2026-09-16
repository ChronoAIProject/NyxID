use chrono::{DateTime, Duration, Utc};
use mongodb::{
    Database,
    bson::{self, doc},
};
use uuid::Uuid;
use zeroize::Zeroizing;

use super::{
    api_key_credential_service as credentials, api_key_mutation_service as mutations,
    audit_service, auth_agent_key_login_service as agent, auth_device_service as device,
    login_client_context::sanitize_context, token_service,
};
use crate::{
    config::AppConfig,
    crypto::aes::EncryptionKeys,
    crypto::jwt::JwtKeys,
    errors::{AppError, AppResult},
    models::{
        api_key_credential::CredentialRevokedReason,
        auth_device_code::{AuthDeviceCode, V2_COLLECTION_NAME as DEVICE_CODES},
        login_client_context::LoginClientContext,
        login_code::{COLLECTION_NAME, LoginCode, LoginCodeStatus as Status},
        login_grant::Selection,
    },
};

pub const CODE_TTL_SECS: i64 = 300;

pub struct MintedCode {
    pub id: String,
    pub code: Zeroizing<String>,
    pub expires_at: DateTime<Utc>,
}
pub enum Delivery {
    Account(token_service::IssuedTokens),
    AgentKey(Box<agent::Delivery>),
}

fn hash(key: &[u8], code: &str) -> AppResult<String> {
    let normalized = device::normalize_user_code(code).map_err(|_| AppError::LoginCodeInvalid)?;
    Ok(device::hmac_hex(
        key,
        format!("login-code:{normalized}").as_bytes(),
    ))
}

fn state_error(row: &LoginCode) -> Option<AppError> {
    match row.status {
        Status::Redeemed | Status::Revoked => Some(AppError::LoginCodeRedeemed),
        Status::Cancelled => Some(AppError::LoginCodeCancelled),
        Status::Pending if row.expires_at <= Utc::now() => Some(AppError::LoginCodeExpired),
        Status::Pending => None,
    }
}

pub async fn limit(db: &Database, namespace: &str, key: &str, max: u32) -> AppResult<()> {
    if !crate::mw::rate_limit::PerKeyRateLimiter::with_db(db.clone(), namespace, max, 300)
        .check_shared(key)
        .await?
    {
        audit_service::log_async(
            db.clone(),
            None,
            "login_code_rate_limited".into(),
            Some(serde_json::json!({"boundary": namespace})),
            None,
            None,
            None,
            None,
        );
        return Err(AppError::LoginCodeRateLimited);
    }
    Ok(())
}

pub async fn mint(
    db: &Database,
    key: &[u8],
    actor: &str,
    selection: Option<Selection>,
    expiry: Option<DateTime<Utc>>,
) -> AppResult<MintedCode> {
    limit(db, "login-code-mint-owner", actor, 5).await?;
    if let Some(Selection::Existing { api_key_id }) = &selection {
        let parent = agent::eligible_key(db, actor, api_key_id).await?;
        credentials::credential_expiry(parent.expires_at, expiry)?;
    }
    if let Some(Selection::New(input)) = &selection {
        let parent_expiry = super::key_service::validate_login_api_key(db, actor, input).await?;
        credentials::credential_expiry(parent_expiry, expiry)?;
    }
    if selection.is_none() && expiry.is_some() {
        return Err(AppError::ValidationError(
            "Credential expiry requires an Agent Key grant".into(),
        ));
    }
    for _ in 0..8 {
        let code = Zeroizing::new(device::format_user_code(&device::generate_user_code()));
        let now = Utc::now();
        let row = LoginCode {
            id: Uuid::new_v4().to_string(),
            user_id: actor.into(),
            code_hmac: hash(key, &code)?,
            status: Status::Pending,
            selection: selection.clone(),
            credential_expires_at: expiry,
            device_request_id: None,
            session_id: None,
            credential_id: None,
            context: None,
            redeemed_at: None,
            created_at: now,
            expires_at: now + Duration::seconds(CODE_TTL_SECS),
            purge_at: now + Duration::days(7),
        };
        match db
            .collection::<LoginCode>(COLLECTION_NAME)
            .insert_one(&row)
            .await
        {
            Ok(_) => {
                audit_service::log_async(
                    db.clone(),
                    Some(actor.into()),
                    "login_code_created".into(),
                    Some(
                        serde_json::json!({"request_id": row.id, "auth_kind": if selection.is_some() {"agent_key"} else {"account_session"}}),
                    ),
                    None,
                    None,
                    None,
                    None,
                );
                return Ok(MintedCode {
                    id: row.id,
                    code,
                    expires_at: row.expires_at,
                });
            }
            Err(error) if super::device_code_service::is_duplicate_key_error(&error) => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(AppError::Internal("Could not allocate login code".into()))
}

pub async fn status(db: &Database, actor: &str, id: &str) -> AppResult<LoginCode> {
    db.collection::<LoginCode>(COLLECTION_NAME)
        .find_one(doc! {"_id": id, "user_id": actor})
        .await?
        .ok_or(AppError::LoginCodeInvalid)
}

pub async fn cancel(db: &Database, actor: &str, id: &str) -> AppResult<()> {
    let row = status(db, actor, id).await?;
    if let Some(error) = state_error(&row) {
        return Err(error);
    }
    let updated = db.collection::<LoginCode>(COLLECTION_NAME).update_one(doc! {
        "_id": id, "user_id": actor, "status": "pending", "expires_at": {"$gt": bson::DateTime::from_chrono(Utc::now())}
    }, doc! {"$set": {"status": "cancelled"}}).await?;
    if updated.modified_count == 0 {
        return Err(
            state_error(&status(db, actor, id).await?).unwrap_or(AppError::LoginCodeInvalid)
        );
    }
    audit_service::log_async(
        db.clone(),
        Some(actor.into()),
        "login_code_cancelled".into(),
        Some(serde_json::json!({"request_id": id})),
        None,
        None,
        None,
        None,
    );
    Ok(())
}

pub async fn revoke(
    db: &Database,
    actor: &str,
    id: &str,
    mcp: Option<&crate::models::mcp_session::McpSessionStore>,
) -> AppResult<()> {
    let row = status(db, actor, id).await?;
    if !matches!(row.status, Status::Redeemed | Status::Revoked) {
        return Err(AppError::LoginCodeInvalid);
    }
    if let Some(session) = &row.session_id {
        token_service::revoke_session(db, session, mcp).await?;
    }
    if let Some(child) = &row.credential_id {
        credentials::revoke(db, child, CredentialRevokedReason::WebRevoke).await?;
    }
    db.collection::<LoginCode>(COLLECTION_NAME)
        .update_one(doc! {"_id": id}, doc! {"$set": {"status": "revoked"}})
        .await?;
    audit_service::log_async(
        db.clone(),
        Some(actor.into()),
        "login_code_revoked".into(),
        Some(serde_json::json!({"request_id": id})),
        None,
        None,
        None,
        None,
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn redeem(
    db: &Database,
    config: &AppConfig,
    jwt: &JwtKeys,
    encryption: &EncryptionKeys,
    key: &[u8],
    code: &str,
    context: LoginClientContext,
    profile: Option<&str>,
) -> AppResult<Delivery> {
    let context = sanitize_context(context);
    let source = context
        .client_ip
        .as_deref()
        .ok_or(AppError::LoginCodeInvalid)?;
    limit(db, "login-code-redeem-source", source, 10).await?;
    let row = db
        .collection::<LoginCode>(COLLECTION_NAME)
        .find_one(doc! {"code_hmac": hash(key, code)?})
        .await?;
    let Some(row) = row else {
        audit_service::log_async(
            db.clone(),
            None,
            "login_code_redeem_failed".into(),
            None,
            context.client_ip,
            context.client_user_agent,
            None,
            None,
        );
        return Err(AppError::LoginCodeInvalid);
    };
    limit(db, "login-code-redeem-owner", &row.user_id, 10).await?;
    if let Some(error) = state_error(&row) {
        return Err(error);
    }
    let account = if row.selection.is_none() && row.device_request_id.is_none() {
        Some(
            token_service::prepare_session_tokens(
                db,
                config,
                jwt,
                &row.user_id,
                context.client_ip.as_deref(),
                context.client_user_agent.as_deref(),
            )
            .await?,
        )
    } else {
        None
    };
    let handoff = if let Some(id) = &row.device_request_id {
        let request = handoff_request(db, doc! {"_id": id})
            .await
            .map_err(handoff_redeem_error)?;
        let grant = request
            .agent_key_grant
            .as_ref()
            .ok_or(AppError::LoginCodeInvalid)?;
        Some(
            agent::prepare_credential_delivery(
                db,
                encryption,
                Some(&grant.credential_id),
                Some(&row.user_id),
                grant.delivery_credential_encrypted.as_deref(),
                grant.key_was_created,
            )
            .await?,
        )
    } else {
        None
    };
    let key_id = match &row.selection {
        Some(Selection::Existing { api_key_id }) => api_key_id.clone(),
        _ => Uuid::new_v4().to_string(),
    };
    let child_id = Uuid::new_v4().to_string();
    let secret = credentials::generate_secret();
    let db_owned = db.clone();
    let row_owned = row.clone();
    let context_owned = context.clone();
    let profile = profile.map(str::to_owned);
    let mut transaction = db.client().start_session().await?;
    let result = transaction.start_transaction().and_run2(async move |session| {
        let db = &db_owned; let row = &row_owned; let context = &context_owned;
        let result: AppResult<Delivery> = async {
            let now = Utc::now();
            let claimed = db.collection::<LoginCode>(COLLECTION_NAME).find_one_and_update(doc! {
                "_id": &row.id, "status": "pending", "expires_at": {"$gt": bson::DateTime::from_chrono(now)}
            }, doc! {"$set": {"status": "redeemed", "redeemed_at": bson::DateTime::from_chrono(now),
                "context": bson::to_bson(context).map_err(|_| AppError::Internal("Invalid login context".into()))?}})
                .session(&mut *session).await?;
            if claimed.is_none() {
                return Err(state_error(&status(db, &row.user_id, &row.id).await?).unwrap_or(AppError::LoginCodeRedeemed));
            }
            let (delivery, session_id, credential_id) = if let Some(account) = &account {
                token_service::insert_prepared_session(db, account, &mut *session).await?;
                let tokens = &account.tokens;
                (Delivery::Account(token_service::IssuedTokens {access_token: tokens.access_token.clone(), refresh_token: tokens.refresh_token.clone(),
                    session_id: tokens.session_id.clone(), access_expires_in: tokens.access_expires_in, resource_uris: tokens.resource_uris.clone()}),
                    Some(tokens.session_id.clone()), None)
            } else if let Some(handoff) = &handoff {
                let claimed = db.collection::<AuthDeviceCode>(DEVICE_CODES).update_one(doc! {
                    "_id": row.device_request_id.as_deref(), "status": "approved", "expires_at": {"$gt": bson::DateTime::from_chrono(now)}
                }, doc! {"$set": {"status": "delivered", "delivered_at": bson::DateTime::from_chrono(now),
                    "purge_at": bson::DateTime::from_chrono(now + Duration::days(1))},
                    "$unset": {"agent_key_grant.delivery_credential_encrypted": ""}}).session(&mut *session).await?;
                if claimed.modified_count == 0 {
                    return Err(handoff_claim_error(db, row.device_request_id.as_deref().ok_or(AppError::LoginCodeInvalid)?).await);
                }
                (Delivery::AgentKey(Box::new(agent::Delivery {credential: handoff.credential.clone(), credential_id: handoff.credential_id.clone(),
                    credential_expires_at: handoff.credential_expires_at.clone(), label: handoff.label.clone(), api_key: handoff.api_key.clone()})),
                    None, Some(handoff.credential_id.clone()))
            } else {
                let selection = row.selection.as_ref().ok_or(AppError::LoginCodeInvalid)?;
                let parent = agent::issue_selected(db, &row.user_id, &row.id, context, profile.as_deref(), selection,
                    row.credential_expires_at, &key_id, &child_id, &secret, &mut *session).await?;
                let summary = agent::key_summary(db, &parent, matches!(selection, Selection::New(_))).await?;
                let child = db.collection::<crate::models::api_key_credential::ApiKeyCredential>(crate::models::api_key_credential::COLLECTION_NAME)
                    .find_one(doc! {"_id": &child_id}).session(&mut *session).await?.ok_or(AppError::AgentKeyCredentialNotFound)?;
                (Delivery::AgentKey(Box::new(agent::Delivery {credential: secret.clone(), credential_id: child_id.clone(),
                    credential_expires_at: child.expires_at.map(|v| v.to_rfc3339()), label: child.label, api_key: summary})), None, Some(child_id.clone()))
            };
            db.collection::<LoginCode>(COLLECTION_NAME).update_one(doc! {"_id": &row.id}, doc! {"$set": {
                "session_id": session_id, "credential_id": credential_id
            }}).session(&mut *session).await?;
            Ok(delivery)
        }.await;
        mutations::transaction_result(result)
    }).await.map_err(mutations::map_transaction_error)?;
    audit_service::log_async(
        db.clone(),
        Some(row.user_id),
        "login_code_redeemed".into(),
        Some(serde_json::json!({"request_id": row.id})),
        context.client_ip,
        context.client_user_agent,
        None,
        None,
    );
    Ok(result)
}

/// Replaces only a pending handoff code. A concurrent redemption fences this
/// update through the same approved device row in the transaction.
pub async fn browser_handoff(
    db: &Database,
    key: &[u8],
    device_code: &str,
) -> AppResult<MintedCode> {
    let request = handoff_request(
        db,
        doc! {"device_code_hmac": device::hmac_hex(key, device_code.as_bytes())},
    )
    .await?;
    let actor = request
        .approved_user_id
        .as_deref()
        .ok_or(AppError::LoginCodeInvalid)?;
    let code = Zeroizing::new(device::format_user_code(&device::generate_user_code()));
    let now = Utc::now();
    let expires_at = request.approved_at.unwrap_or(now) + Duration::seconds(CODE_TTL_SECS);
    let row = LoginCode {
        id: Uuid::new_v4().to_string(),
        user_id: actor.into(),
        code_hmac: hash(key, &code)?,
        status: Status::Pending,
        selection: None,
        credential_expires_at: None,
        device_request_id: Some(request.id.clone()),
        session_id: None,
        credential_id: None,
        context: None,
        redeemed_at: None,
        created_at: now,
        expires_at,
        purge_at: now + Duration::days(7),
    };
    let mut transaction = db.client().start_session().await?;
    let db_owned = db.clone();
    let row_owned = row.clone();
    transaction.start_transaction().and_run2(async move |session| {
        let db = &db_owned; let row = &row_owned;
        let result: AppResult<()> = async {
            let claimed = db.collection::<AuthDeviceCode>(DEVICE_CODES).update_one(doc! {"_id": &request.id,
                "status": "approved", "expires_at": {"$gt": bson::DateTime::from_chrono(Utc::now())}},
                doc! {"$set": {"expires_at": bson::DateTime::from_chrono(expires_at)}, "$inc": {"handoff_version": 1}}).session(&mut *session).await?;
            if claimed.matched_count == 0 {
                return Err(match handoff_request(db, doc! {"_id": &request.id}).await {
                    Err(error) => error,
                    Ok(_) => AppError::Internal("Device handoff claim lost without a terminal outcome".into()),
                });
            }
            db.collection::<LoginCode>(COLLECTION_NAME).delete_many(doc! {"device_request_id": &request.id, "status": "pending"})
                .session(&mut *session).await?;
            db.collection::<LoginCode>(COLLECTION_NAME).insert_one(row).session(&mut *session).await?;
            Ok(())
        }.await;
        mutations::transaction_result(result)
    }).await.map_err(mutations::map_transaction_error)?;
    Ok(MintedCode {
        id: row.id,
        code,
        expires_at,
    })
}

async fn handoff_request(db: &Database, filter: bson::Document) -> AppResult<AuthDeviceCode> {
    use crate::models::auth_device_code::AuthDeviceCodeStatus as DeviceStatus;
    let row = db
        .collection::<AuthDeviceCode>(DEVICE_CODES)
        .find_one(filter)
        .await?
        .ok_or(AppError::AuthDeviceCodeExpired)?;
    match row.status {
        DeviceStatus::Delivered => return Err(AppError::AuthDeviceCodeAlreadyDelivered),
        DeviceStatus::Denied => return Err(AppError::AuthDeviceCodeDenied),
        DeviceStatus::Expired => return Err(AppError::AuthDeviceCodeExpired),
        _ => {}
    }
    if row.expires_at <= Utc::now() {
        return Err(AppError::AuthDeviceCodeExpired);
    }
    if row.status != DeviceStatus::Approved {
        return Err(AppError::AuthDeviceCodePending);
    }
    if row.agent_key_grant.is_none() {
        return Err(AppError::LoginCodeInvalid);
    }
    Ok(row)
}

fn handoff_redeem_error(error: AppError) -> AppError {
    match error {
        AppError::AuthDeviceCodeAlreadyDelivered => AppError::LoginCodeRedeemed,
        AppError::AuthDeviceCodeExpired => AppError::LoginCodeExpired,
        AppError::AuthDeviceCodeDenied | AppError::AuthDeviceCodePending => {
            AppError::LoginCodeInvalid
        }
        error => error,
    }
}

async fn handoff_claim_error(db: &Database, id: &str) -> AppError {
    match handoff_request(db, doc! {"_id": id}).await {
        Err(error) => handoff_redeem_error(error),
        Ok(_) => AppError::Internal("Device handoff claim lost without a terminal outcome".into()),
    }
}

#[cfg(test)]
mod tests;
