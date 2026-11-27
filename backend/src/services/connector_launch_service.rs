use chrono::{Duration, Utc};
use mongodb::bson::{DateTime, Document, doc};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::crypto::aes::{EncryptionKeys, RecordContext};
use crate::crypto::token::{constant_time_eq, hash_token};
use crate::errors::{AppError, AppResult};
use crate::models::oauth_state::{COLLECTION_NAME, OAuthState, PendingOAuthCallback};

pub const CALLBACK_PATH: &str = "/api/v1/providers/callback";

pub fn cookie_name(state_id: &str) -> String {
    // Cookie names are fixed ASCII and per attempt, so simultaneous browser
    // launches cannot overwrite one another's proof.
    format!("nyx_connector_{}", &hash_token(state_id)[..24])
}

pub fn matching_proof(state: &OAuthState, cookie: Option<&str>) -> Option<String> {
    let (Some(expected), Some(cookie)) = (&state.launch_proof_hash, cookie) else {
        return None;
    };
    let actual = hash_token(cookie);
    constant_time_eq(expected.as_bytes(), actual.as_bytes()).then_some(actual)
}

fn available_state() -> Document {
    doc! {
        "consumed": {"$ne": true},
        "expires_at": {"$gt": DateTime::now()},
        "device_code_encrypted": null,
    }
}

fn unbound_state(id: &str) -> Document {
    let mut filter = available_state();
    filter.insert("_id", id);
    filter.insert("pending_callback", mongodb::bson::Bson::Null);
    filter
}

pub async fn claim_browser(
    db: &mongodb::Database,
    state: &OAuthState,
    proof_hash: &str,
) -> AppResult<OAuthState> {
    let mut filter = unbound_state(&state.id);
    filter.insert("launch_proof_hash", proof_hash);
    claim(db, filter).await
}

pub async fn stage(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    state: &OAuthState,
    code: &str,
) -> AppResult<String> {
    if code.is_empty() || code.len() > 8192 {
        return Err(invalid_launch());
    }
    let continuation_id = Uuid::new_v4().to_string();
    let context_fields = [
        "pending_callback",
        &state.provider_config_id,
        &continuation_id,
    ];
    let context = RecordContext::new(COLLECTION_NAME, &state.id, &state.user_id, &context_fields)?;
    let pending = PendingOAuthCallback {
        id: continuation_id.clone(),
        encrypted_code: hex::encode(keys.encrypt_record(code.as_bytes(), &context).await?),
        expires_at: (Utc::now() + Duration::minutes(5)).min(state.expires_at),
    };
    let pending = mongodb::bson::to_document(&pending)
        .map_err(|_| AppError::Internal("Could not stage connector authorization".into()))?;
    let result = super::transaction_service::durable_database(db)
        .collection::<OAuthState>(COLLECTION_NAME)
        .update_one(
            unbound_state(&state.id),
            doc! {"$set": {"pending_callback": pending}},
        )
        .await?;
    if result.modified_count != 1 {
        return Err(invalid_launch());
    }
    Ok(continuation_id)
}

fn continuation_filter(id: &str, user_id: &str) -> AppResult<Document> {
    if id.len() != 36 || Uuid::parse_str(id).is_err() {
        return Err(invalid_launch());
    }
    let mut filter = available_state();
    filter.insert("user_id", user_id);
    filter.insert("pending_callback.id", id);
    filter.insert("pending_callback.expires_at", doc! {"$gt": DateTime::now()});
    Ok(filter)
}

pub async fn preview(db: &mongodb::Database, id: &str, user_id: &str) -> AppResult<OAuthState> {
    db.collection::<OAuthState>(COLLECTION_NAME)
        .find_one(continuation_filter(id, user_id)?)
        .await?
        .ok_or_else(invalid_launch)
}

pub async fn confirm(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    id: &str,
    user_id: &str,
) -> AppResult<(OAuthState, Zeroizing<String>)> {
    let state = claim(db, continuation_filter(id, user_id)?).await?;
    let pending = state.pending_callback.as_ref().ok_or_else(invalid_launch)?;
    let context_fields = ["pending_callback", &state.provider_config_id, &pending.id];
    let context = RecordContext::new(COLLECTION_NAME, &state.id, &state.user_id, &context_fields)?;
    let encrypted = hex::decode(&pending.encrypted_code).map_err(|_| invalid_launch())?;
    let plaintext = keys.decrypt_record(&encrypted, &context).await?;
    let code = std::str::from_utf8(&plaintext).map_err(|_| invalid_launch())?;
    Ok((state, Zeroizing::new(code.to_owned())))
}

async fn claim(db: &mongodb::Database, filter: Document) -> AppResult<OAuthState> {
    super::transaction_service::durable_database(db)
        .collection::<OAuthState>(COLLECTION_NAME)
        .find_one_and_update(
            filter,
            doc! {"$set": {"consumed": true}, "$unset": {"pending_callback": ""}},
        )
        .await?
        .ok_or_else(invalid_launch)
}

fn invalid_launch() -> AppError {
    AppError::NotFound(
        "This connection request has expired or is unavailable to your account.".into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oauth_state(user_id: &str, launch: Option<&str>) -> OAuthState {
        let mut state = mongodb::bson::from_document::<OAuthState>(doc! {
            "_id": Uuid::new_v4().to_string(),
            "user_id": user_id,
            "provider_config_id": Uuid::new_v4().to_string(),
            "expires_at": DateTime::from_chrono(Utc::now() + Duration::minutes(10)),
            "created_at": DateTime::now(),
        })
        .unwrap();
        state.launch_proof_hash = launch.map(hash_token);
        state
    }

    #[tokio::test]
    async fn connector_launch_claim_requires_matching_proof_and_one_winner() {
        let db =
            crate::test_utils::connect_transaction_test_database("connector_launch_cookie").await;
        let user_id = Uuid::new_v4().to_string();
        let state = oauth_state(&user_id, Some("browser-proof"));
        db.collection::<OAuthState>(COLLECTION_NAME)
            .insert_one(&state)
            .await
            .unwrap();
        assert!(matching_proof(&state, None).is_none());
        assert!(matching_proof(&state, Some("different-proof")).is_none());
        assert!(
            claim_browser(&db, &state, &hash_token("different-proof"))
                .await
                .is_err()
        );
        let proof = matching_proof(&state, Some("browser-proof")).unwrap();
        let (first, second) = tokio::join!(
            claim_browser(&db, &state, &proof),
            claim_browser(&db, &state, &proof),
        );
        assert_ne!(first.is_ok(), second.is_ok());
    }

    #[tokio::test]
    async fn connector_launch_legacy_continuation_is_encrypted_owner_bound_and_one_time() {
        let db =
            crate::test_utils::connect_transaction_test_database("connector_launch_legacy").await;
        let app = crate::test_utils::test_app_state(db.clone());
        let user_id = Uuid::new_v4().to_string();
        let other_id = Uuid::new_v4().to_string();
        let state = oauth_state(&user_id, None);
        let rows = db.collection::<OAuthState>(COLLECTION_NAME);
        rows.insert_one(&state).await.unwrap();
        assert!(matching_proof(&state, Some("old-browser")).is_none());
        let (first, second) = tokio::join!(
            stage(&db, &app.encryption_keys, &state, "provider-code"),
            stage(&db, &app.encryption_keys, &state, "provider-code"),
        );
        assert_ne!(first.is_ok(), second.is_ok());
        let id = first.or(second).unwrap();
        let staged = preview(&db, &id, &user_id).await.unwrap();
        assert!(!staged.consumed);
        assert_ne!(
            staged.pending_callback.unwrap().encrypted_code,
            "provider-code"
        );
        assert!(preview(&db, &id, &other_id).await.is_err());
        assert!(
            confirm(&db, &app.encryption_keys, &id, &other_id)
                .await
                .is_err()
        );
        assert_eq!(
            confirm(&db, &app.encryption_keys, &id, &user_id)
                .await
                .unwrap()
                .1
                .as_str(),
            "provider-code"
        );
        assert!(
            confirm(&db, &app.encryption_keys, &id, &user_id)
                .await
                .is_err()
        );
        let saved = rows
            .find_one(doc! {"_id": &state.id})
            .await
            .unwrap()
            .unwrap();
        assert!(saved.consumed && saved.pending_callback.is_none());

        let mut expired = oauth_state(&user_id, None);
        expired.expires_at = Utc::now() - Duration::minutes(1);
        rows.insert_one(&expired).await.unwrap();
        assert!(
            stage(&db, &app.encryption_keys, &expired, "provider-code")
                .await
                .is_err()
        );
    }
}
