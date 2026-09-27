use chrono::{DateTime, Duration, Utc};
use mongodb::{Database, bson::doc, options::ReturnDocument};
use uuid::Uuid;
use zeroize::Zeroizing;

use super::{
    auth_agent_key_login_service as agent, auth_device_service as device, auth_service,
    token_service,
};
use crate::{
    crypto::token::{constant_time_eq, generate_random_token, hash_token},
    errors::{AppError, AppResult},
    models::{
        auth_device_code::AuthDeviceCodeStatus,
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
    if row.verified && user.mfa_enabled && !row.mfa_verified {
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
    auth_service::ensure_person_user(actor)?;
    if !actor.is_active {
        return Err(invalid());
    }
    db.collection::<LoginApproval>(COLLECTION_NAME).find_one_and_update(
        doc! {"_id": &row.id, "user_id": null, "closed": false, "verified": false, "expires_at": {"$gt": mongodb::bson::DateTime::now()}},
        doc! {"$set": {"user_id": &actor.id, "social_state_hash": null}})
        .return_document(ReturnDocument::After).await?.ok_or_else(invalid)
}

pub async fn verify(
    db: &Database,
    row: &LoginApproval,
    mfa_verified: bool,
    ip: &str,
    user_agent: Option<&str>,
) -> AppResult<(LoginApproval, Option<token_service::IssuedSession>)> {
    let db = db.clone();
    let row = row.clone();
    let ip = ip.to_owned();
    let user_agent = user_agent.map(str::to_owned);
    let mut transaction = db.client().start_session().await?;
    transaction.start_transaction().and_run2(async move |session| {
        let result: AppResult<_> = async {
            let verified = db.collection::<LoginApproval>(COLLECTION_NAME).find_one_and_update(
                doc! {"_id": &row.id, "user_id": &row.user_id, "verified": false, "closed": false, "expires_at": {"$gt": mongodb::bson::DateTime::now()}},
                doc! {"$set": {"verified": true, "mfa_verified": mfa_verified}})
                .return_document(ReturnDocument::After).session(&mut *session).await?.ok_or_else(invalid)?;
            let browser = if verified.keep_signed_in {
                Some(token_service::create_session_with_transaction(&db,
                    verified.user_id.as_deref().ok_or_else(invalid)?, Some(&ip), user_agent.as_deref(), Some(&mut *session)).await?)
            } else { None };
            Ok((verified, browser))
        }.await;
        super::api_key_mutation_service::transaction_result(result)
    }).await.map_err(super::api_key_mutation_service::map_transaction_error)
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
