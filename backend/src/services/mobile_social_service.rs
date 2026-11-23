use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use bson::{Binary, doc, spec::BinarySubtype};
use chrono::{Duration, Utc};
use mongodb::{Database, options::WriteConcern};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    crypto::{
        aes::EncryptionKeys,
        token::{generate_random_token, hash_token},
    },
    errors::{AppError, AppResult},
    models::mobile_social::{MobileSocialDelivery, MobileSocialFlow},
    services::{
        oauth_flow,
        token_service::{PreparedSessionRows, PreparedSessionTokens},
    },
};

pub(crate) const STATE_PREFIX: &str = "mobile1_";
pub(crate) const FLOW_SECONDS: i64 = 600;

pub(crate) async fn start(
    db: &Database,
    state: &str,
    provider: &str,
    origin: &str,
    redirect_uri: &str,
    nonce: Option<&str>,
    code_challenge: Option<&str>,
) -> AppResult<()> {
    let flow = MobileSocialFlow {
        id: Uuid::new_v4().to_string(),
        state_hash: hash_token(state),
        provider: provider.into(),
        callback_origin: origin.into(),
        redirect_uri: redirect_uri.into(),
        nonce_hash: nonce.map(hash_token),
        code_challenge: code_challenge.map(str::to_owned),
        expires_at: Utc::now() + Duration::seconds(FLOW_SECONDS),
    };
    db.collection::<MobileSocialFlow>(MobileSocialFlow::COLLECTION_NAME)
        .insert_one(flow)
        .write_concern(WriteConcern::majority())
        .await?;
    Ok(())
}

pub(crate) async fn consume(
    db: &Database,
    state: &str,
    provider: &str,
    origin: &str,
) -> AppResult<Option<MobileSocialFlow>> {
    Ok(db
        .collection::<MobileSocialFlow>(MobileSocialFlow::COLLECTION_NAME)
        .find_one_and_delete(doc! {
            "state_hash": hash_token(state), "provider": provider, "callback_origin": origin,
            "$expr": {"$gt": ["$expires_at", "$$NOW"]}
        })
        .write_concern(WriteConcern::majority())
        .await?)
}

pub(crate) fn valid_challenge(challenge: &str) -> bool {
    challenge.len() == 43
        && URL_SAFE_NO_PAD
            .decode(challenge)
            .is_ok_and(|bytes| bytes.len() == 32)
}

#[derive(Serialize, Deserialize)]
pub(crate) struct TokenDelivery {
    pub access_token: Zeroizing<String>,
    pub refresh_token: Zeroizing<String>,
    pub expires_in: i64,
}

#[derive(Serialize, Deserialize)]
struct PendingDelivery {
    tokens: TokenDelivery,
    rows: PreparedSessionRows,
}

crate::errors::redacted_debug!(TokenDelivery, PendingDelivery);

pub(crate) async fn issue_code(
    db: &Database,
    keys: &EncryptionKeys,
    prepared: PreparedSessionTokens,
    user_id: &str,
    challenge: &str,
) -> AppResult<String> {
    let code = generate_random_token();
    let code_hash = hash_token(&code);
    let pending = PendingDelivery {
        tokens: TokenDelivery {
            access_token: Zeroizing::new(prepared.tokens.access_token),
            refresh_token: Zeroizing::new(prepared.tokens.refresh_token),
            expires_in: prepared.tokens.access_expires_in,
        },
        rows: prepared.rows,
    };
    let payload = Zeroizing::new(
        bson::to_vec(&pending)
            .map_err(|_| AppError::Internal("Could not encode mobile sign-in response".into()))?,
    );
    let aad = format!("mobile-social-delivery-v1:{code_hash}:{challenge}:{user_id}");
    let bytes = keys.encrypt_with_aad(&payload, aad.as_bytes()).await?;
    let delivery = MobileSocialDelivery {
        id: Uuid::new_v4().to_string(),
        code_hash,
        code_challenge: challenge.into(),
        user_id: user_id.into(),
        ciphertext: Binary {
            subtype: BinarySubtype::Generic,
            bytes,
        },
        expires_at: Utc::now() + Duration::seconds(60),
    };
    super::erasure_admission::insert_owned(
        db,
        MobileSocialDelivery::COLLECTION_NAME,
        user_id,
        &delivery,
    )
    .await?;
    Ok(code)
}

fn invalid_code() -> AppError {
    AppError::ValidationError("Invalid or expired mobile sign-in code".into())
}

pub(crate) async fn redeem(
    db: &Database,
    keys: &EncryptionKeys,
    code: &str,
    verifier: &str,
) -> AppResult<TokenDelivery> {
    if code.len() != 64
        || !code.bytes().all(|b| b.is_ascii_hexdigit())
        || !(43..=128).contains(&verifier.len())
        || !verifier
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
    {
        return Err(invalid_code());
    }
    let challenge = oauth_flow::generate_code_challenge(verifier);
    let code_hash = hash_token(code);
    let filter = doc! {"code_hash": &code_hash, "code_challenge": &challenge,
    "$expr": {"$gt": ["$expires_at", "$$NOW"]}};
    let deliveries = db.collection::<MobileSocialDelivery>(MobileSocialDelivery::COLLECTION_NAME);
    let row = deliveries
        .find_one(filter.clone())
        .await?
        .ok_or_else(invalid_code)?;
    let aad = format!(
        "mobile-social-delivery-v1:{code_hash}:{challenge}:{}",
        row.user_id
    );
    let payload = Zeroizing::new(
        keys.decrypt_with_aad(&row.ciphertext.bytes, aad.as_bytes())
            .await?,
    );
    let pending: PendingDelivery = bson::from_slice(&payload)
        .map_err(|_| AppError::Internal("Invalid stored mobile sign-in response".into()))?;
    let mut claim_filter = filter;
    claim_filter.insert("_id", row.id);
    super::transaction_service::run_db_only(
        db,
        &(db, deliveries, claim_filter, pending.rows),
        |tx, (db, deliveries, filter, rows)| {
            Box::pin(async move {
                if deliveries
                    .delete_one(filter.clone())
                    .session(&mut *tx)
                    .await?
                    .deleted_count
                    == 0
                {
                    return Err(invalid_code());
                }
                rows.insert(db, tx).await
            })
        },
    )
    .await?;
    Ok(pending.tokens)
}
