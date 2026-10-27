use chrono::{DateTime, Duration, Utc};
use mongodb::{Database, bson::doc, options::ReturnDocument};
use uuid::Uuid;
use zeroize::Zeroizing;

use super::{
    approved_sign_in_policy as app_policy, auth_agent_key_login_service as agent,
    auth_device_service as device, auth_service, token_service,
};
use crate::{
    crypto::token::{constant_time_eq, generate_random_token, hash_token},
    errors::{AppError, AppResult},
    models::{
        auth_device_code::{AuthDeviceCode, AuthDeviceCodeStatus, COLLECTION_NAME as DEVICE_CODES},
        login_approval::{COLLECTION_NAME, LoginApproval, LoginFlow},
        user::{COLLECTION_NAME as USERS, User},
    },
};

pub fn invalid() -> AppError {
    AppError::Unauthorized("This request verification ended. Verify your identity again.".into())
}

pub async fn request_binding(
    db: &Database,
    key: &[u8],
    flow: LoginFlow,
    code: &str,
) -> AppResult<(String, DateTime<Utc>)> {
    let (id, expiry, pending) = match flow {
        LoginFlow::Device => {
            let code = device::normalize_user_code(code)?;
            let (_, row) =
                device::find_by_user_code(db, &device::hmac_hex(key, code.as_bytes())).await?;
            (
                row.id,
                row.expires_at,
                row.status == AuthDeviceCodeStatus::Pending,
            )
        }
        LoginFlow::AgentKey => {
            let row = agent::find_by_user_code(db, key, code).await?;
            (
                row.id,
                row.expires_at,
                row.status == crate::models::agent_key_login_request::AgentKeyLoginStatus::Pending,
            )
        }
    };
    if !pending || expiry <= Utc::now() {
        return Err(invalid());
    }
    Ok((id, expiry))
}

pub async fn begin(
    db: &Database,
    key: &[u8],
    flow: LoginFlow,
    code: &str,
    keep_signed_in: bool,
) -> AppResult<(LoginApproval, Zeroizing<String>)> {
    let code = device::normalize_user_code(code)?;
    let (request_id, expires_at) = request_binding(db, key, flow, &code).await?;
    let secret = Zeroizing::new(generate_random_token());
    let row = LoginApproval {
        mfa_policy_revision: None,
        id: Uuid::new_v4().to_string(),
        browser_hash: hash_token(&secret),
        flow,
        user_code: code,
        request_id,
        keep_signed_in,
        user_id: None,
        verified: false,
        mfa_verified: false,
        closed: false,
        social_state_hash: None,
        attempts: 0,
        expires_at: expires_at.min(Utc::now() + Duration::minutes(10)),
    };
    db.collection::<LoginApproval>(COLLECTION_NAME)
        .insert_one(&row)
        .await?;
    Ok((row, secret))
}

pub async fn load(db: &Database, key: &[u8], id: &str, secret: &str) -> AppResult<LoginApproval> {
    Uuid::parse_str(id).map_err(|_| invalid())?;
    let row = db.collection::<LoginApproval>(COLLECTION_NAME)
        .find_one(doc! {"_id": id, "closed": false, "expires_at": {"$gt": mongodb::bson::DateTime::now()}})
        .await?.ok_or_else(invalid)?;
    if !constant_time_eq(row.browser_hash.as_bytes(), hash_token(secret).as_bytes()) {
        return Err(invalid());
    }
    live(db, key, &row).await?;
    Ok(row)
}

pub async fn live(db: &Database, key: &[u8], row: &LoginApproval) -> AppResult<()> {
    if row.closed || row.expires_at <= Utc::now() {
        return Err(invalid());
    }
    let (id, _) = request_binding(db, key, row.flow, &row.user_code).await?;
    if id != row.request_id {
        return Err(invalid());
    }
    Ok(())
}

pub async fn user(db: &Database, row: &LoginApproval) -> AppResult<User> {
    let id = row.user_id.as_deref().ok_or_else(invalid)?;
    let user = db
        .collection::<User>(USERS)
        .find_one(doc! {"_id": id, "is_active": true})
        .await?
        .ok_or_else(invalid)?;
    auth_service::ensure_person_user(&user)?;
    if row.verified
        && !app_policy::permits(&user, row.mfa_verified, row.mfa_policy_revision.as_deref())
    {
        return Err(invalid());
    }
    Ok(user)
}

pub async fn attempt(db: &Database, row: &LoginApproval) -> AppResult<()> {
    let result = db.collection::<LoginApproval>(COLLECTION_NAME).update_one(
        doc! {"_id": &row.id, "closed": false, "verified": false, "attempts": {"$lt": 10}, "expires_at": {"$gt": mongodb::bson::DateTime::now()}},
        doc! {"$inc": {"attempts": 1}}).await?;
    if result.matched_count != 1 {
        return Err(invalid());
    }
    Ok(())
}

/// Bind the first verified password/provider identity; a later completion cannot replace it.
pub async fn identify(
    db: &Database,
    row: &LoginApproval,
    actor: &User,
) -> AppResult<LoginApproval> {
    identify_proof(db, row, actor, false).await
}
pub async fn identify_password(
    db: &Database,
    row: &LoginApproval,
    actor: &User,
) -> AppResult<LoginApproval> {
    identify_proof(db, row, actor, true).await
}
async fn identify_proof(
    db: &Database,
    row: &LoginApproval,
    actor: &User,
    password: bool,
) -> AppResult<LoginApproval> {
    auth_service::ensure_person_user(actor)?;
    if !actor.is_active {
        return Err(invalid());
    }
    let mut transaction = db.client().start_session().await?;
    transaction
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    let result: AppResult<LoginApproval> = async {
    if password {
        auth_service::fence_password_proof(db, actor, &mut transaction).await?;
    }
    let row=db.collection::<LoginApproval>(COLLECTION_NAME).find_one_and_update(
        doc! {"_id": &row.id, "user_id": null, "closed": false, "verified": false, "expires_at": {"$gt": mongodb::bson::DateTime::now()}},
        doc! {"$set": {"user_id": &actor.id, "social_state_hash": null}})
        .return_document(ReturnDocument::After).session(&mut transaction).await?.ok_or_else(invalid)?;
    Ok(row)
    }.await;
    let row = match result {
        Ok(row) => row,
        Err(error) => {
            transaction.abort_transaction().await?;
            return Err(error);
        }
    };
    super::transaction_service::commit(&mut transaction).await?;
    Ok(row)
}

/// Complete a browser approval inside an already fenced MFA transaction.
pub(crate) async fn verify_identity_in_transaction(
    db: &Database,
    row: &LoginApproval,
    mfa_verified: bool,
    ip: &str,
    user_agent: Option<&str>,
    app_request: Option<&AuthDeviceCode>,
    session: &mut mongodb::ClientSession,
) -> AppResult<(LoginApproval, Option<token_service::IssuedSession>)> {
    let revision = app_request.and_then(|request| request.mfa_policy_revision.as_deref());
    app_policy::fence(
        db,
        row.user_id.as_deref().ok_or_else(invalid)?,
        revision,
        session,
    )
    .await?;
    if let Some(request) = app_request {
        let id = &request.id;
        let claimed = db.collection::<AuthDeviceCode>(DEVICE_CODES).update_one(
                    doc! {"_id": id, "login_approval_id": &row.id, "approved_user_id": &row.user_id,
                        "status": "approved", "approved_session_id": null,
                        "mfa_policy_revision": revision, "mfa_assured": mfa_verified,
                        "expires_at": {"$gt": mongodb::bson::DateTime::now()}},
                    doc! {"$set": {"status": "delivered", "delivered_at": mongodb::bson::DateTime::now(),
                        "purge_at": mongodb::bson::DateTime::from_chrono(Utc::now() + Duration::days(1))}})
                    .session(&mut *session).await?;
        if claimed.modified_count != 1 {
            return Err(AppError::AuthDeviceCodeAlreadyDelivered);
        }
    }
    let verified = db.collection::<LoginApproval>(COLLECTION_NAME).find_one_and_update(
                doc! {"_id": &row.id, "user_id": &row.user_id, "verified": false, "closed": false, "expires_at": {"$gt": mongodb::bson::DateTime::now()}},
                doc! {"$set": {"verified": true, "mfa_verified": mfa_verified, "mfa_policy_revision": revision}})
                .return_document(ReturnDocument::After).session(&mut *session).await?.ok_or_else(invalid)?;
    let browser = if verified.keep_signed_in {
        let browser = token_service::create_session_with_transaction_and_mfa_authority(
            db,
            verified.user_id.as_deref().ok_or_else(invalid)?,
            Some(ip),
            user_agent,
            Some(&mut *session),
            mfa_verified,
            revision.map(|_| crate::models::session::MfaExemption::ApprovedSignInPolicy),
        )
        .await?;
        Some(browser)
    } else {
        None
    };
    Ok((verified, browser))
}

pub async fn begin_app(
    db: &Database,
    key: &[u8],
    row: &LoginApproval,
    context: device::InitiateInput,
) -> AppResult<device::InitiateOutput> {
    live(db, key, row).await?;
    if row.user_id.is_some() || row.verified {
        return Err(invalid());
    }
    attempt(db, row).await?;
    let request = device::initiate(db, key, context).await?;
    let linked = db.collection::<AuthDeviceCode>(DEVICE_CODES).update_one(
        doc! {"device_code_hmac": device::hmac_hex(key, request.device_code.as_bytes()), "status": "pending"},
        doc! {"$set": {"login_approval_id": &row.id, "expires_at": mongodb::bson::DateTime::from_chrono(row.expires_at)}})
        .await?;
    if linked.modified_count != 1 {
        return Err(invalid());
    }
    Ok(device::InitiateOutput {
        expires_in: (row.expires_at - Utc::now()).num_seconds().max(0),
        ..request
    })
}

pub async fn approve_app_identity(
    db: &Database,
    key: &[u8],
    request: &AuthDeviceCode,
    input: &device::ApproveInput,
    mfa_assured: bool,
    revision: Option<&str>,
    authority: super::session_authority::DerivedMfaAuthority,
) -> AppResult<()> {
    let id = request.login_approval_id.as_deref().ok_or_else(invalid)?;
    let approval = db
        .collection::<LoginApproval>(COLLECTION_NAME)
        .find_one(doc! {"_id": id, "closed": false, "user_id": null, "verified": false})
        .await?
        .ok_or_else(invalid)?;
    live(db, key, &approval).await?;
    let actor = db
        .collection::<User>(USERS)
        .find_one(doc! {"_id": &input.user_id, "is_active": true})
        .await?
        .ok_or_else(invalid)?;
    auth_service::ensure_person_user(&actor)?;
    if !app_policy::permits(&actor, mfa_assured, revision) {
        return Err(AppError::MfaStepUpRequired);
    }
    let context = (
        db.clone(),
        request.clone(),
        actor.id.clone(),
        revision.map(str::to_owned),
        input.clone(),
        Zeroizing::new(key.to_vec()),
    );
    super::transaction_service::run_db_only(db, &context, move |tx, c| Box::pin(async move {
        let (db, request, actor, revision, input, key) = c;
        app_policy::fence_approval(db, actor, authority, revision.as_deref(), tx).await?;
        let result = db.collection::<AuthDeviceCode>(DEVICE_CODES).update_one(
            doc! {"_id": &request.id, "status": "pending", "expires_at": {"$gt": mongodb::bson::DateTime::now()}},
            doc! {"$set": {"status": "approved", "approved_user_id": actor,
                "approved_at": mongodb::bson::DateTime::now(), "mfa_assured": mfa_assured,
                "mfa_policy_revision": revision,
                "approver_ip_hmac": input.approver_ip.as_deref().map(|ip| device::hmac_hex(key, ip.as_bytes()))}})
            .session(&mut *tx).await?;
        if result.modified_count != 1 { return Err(AppError::AuthDeviceCodeAlreadyDelivered); }
        Ok(())
    })).await?;
    super::audit_service::log_async(
        db.clone(),
        Some(actor.id),
        "login_request_app_identity_approved".into(),
        Some(
            serde_json::json!({"request_id": request.id, "approval_id": id, "mfa_policy": revision.map(|_| "approved_sign_in"), "policy_default": revision == Some(app_policy::DEFAULT_REVISION)}),
        ),
        input.approver_ip.clone(),
        input.approver_user_agent.clone(),
        None,
        None,
    );
    Ok(())
}

pub async fn app_identity(
    db: &Database,
    key: &[u8],
    row: &LoginApproval,
    device_code: &str,
) -> AppResult<(LoginApproval, AuthDeviceCode)> {
    live(db, key, row).await?;
    let request = db.collection::<AuthDeviceCode>(DEVICE_CODES)
        .find_one(doc! {"device_code_hmac": device::hmac_hex(key, device_code.as_bytes()), "login_approval_id": &row.id})
        .await?.ok_or(AppError::AuthDeviceCodeNotFound)?;
    match request.status {
        AuthDeviceCodeStatus::Denied => return Err(AppError::AuthDeviceCodeDenied),
        AuthDeviceCodeStatus::Delivered => return Err(AppError::AuthDeviceCodeAlreadyDelivered),
        _ if request.expires_at <= Utc::now() => return Err(AppError::AuthDeviceCodeExpired),
        AuthDeviceCodeStatus::Pending => return Err(AppError::AuthDeviceCodePending),
        AuthDeviceCodeStatus::Expired => return Err(AppError::AuthDeviceCodeExpired),
        AuthDeviceCodeStatus::Approved => {}
    }
    // An older replica may have approved this as an ordinary account-token request.
    if request.approved_session_id.is_some() {
        return Err(invalid());
    }
    let actor = db
        .collection::<User>(USERS)
        .find_one(doc! {"_id": &request.approved_user_id, "is_active": true})
        .await?
        .ok_or_else(invalid)?;
    if request.mfa_policy_revision.is_none()
        && !super::session_authority::may_derive(actor.mfa_enabled, request.mfa_assured)
    {
        return Err(invalid());
    }
    device::require_delivery_mfa(db, &request).await?;
    let row = if row.user_id.as_deref() == Some(&actor.id) && !row.verified {
        row.clone()
    } else {
        identify(db, row, &actor).await?
    };
    Ok((row, request))
}

pub async fn close(db: &Database, row: &LoginApproval) -> AppResult<()> {
    let result = db
        .collection::<LoginApproval>(COLLECTION_NAME)
        .update_one(
            doc! {"_id": &row.id, "closed": false},
            doc! {"$set": {"closed": true}},
        )
        .await?;
    if result.matched_count != 1 {
        return Err(invalid());
    }
    Ok(())
}

pub async fn retry_decision(db: &Database, row: &LoginApproval) -> AppResult<()> {
    db.collection::<LoginApproval>(COLLECTION_NAME)
        .update_one(doc! {"_id": &row.id}, doc! {"$set": {"closed": false}})
        .await?;
    Ok(())
}

pub async fn bind_social(db: &Database, row: &LoginApproval, state: &str) -> AppResult<()> {
    let result = db
        .collection::<LoginApproval>(COLLECTION_NAME)
        .update_one(
            doc! {"_id": &row.id, "user_id": null, "closed": false, "verified": false},
            doc! {"$set": {"social_state_hash": hash_token(state)}},
        )
        .await?;
    if result.matched_count != 1 {
        return Err(invalid());
    }
    Ok(())
}

pub async fn consume_social(
    db: &Database,
    key: &[u8],
    id: &str,
    state: &str,
) -> AppResult<LoginApproval> {
    let row = db.collection::<LoginApproval>(COLLECTION_NAME).find_one_and_update(
        doc! {"_id": id, "social_state_hash": hash_token(state), "user_id": null, "closed": false, "verified": false, "expires_at": {"$gt": mongodb::bson::DateTime::now()}},
        doc! {"$set": {"social_state_hash": null}}).return_document(ReturnDocument::After).await?.ok_or_else(invalid)?;
    live(db, key, &row).await?;
    Ok(row)
}
