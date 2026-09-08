use super::*;
use crate::models::api_key_credential::CredentialRevokedReason;
use crate::services::{
    api_key_credential_service as credentials, api_key_mutation_service as mutations,
    auth_agent_key_login_service as agent,
};
use futures::TryStreamExt;

async fn pending_request(db: &Database, hmac_key: &[u8], code: &str) -> AppResult<AuthDeviceCode> {
    let normalized = normalize_user_code(code)?;
    if !is_v2_user_code(&normalized) {
        return Err(AppError::ValidationError("This legacy request supports account login only. Start a new login with an updated CLI.".into()));
    }
    let row = collection_for_user_code(db, &normalized)
        .find_one(doc! {"user_code_hmac": hmac_hex(hmac_key, normalized.as_bytes())})
        .sort(doc! {"created_at": -1})
        .await?
        .ok_or(AppError::AuthDeviceUserCodeInvalid)?;
    if row.status != AuthDeviceCodeStatus::Pending {
        return Err(non_pending_approve_error(row.status));
    }
    if row.expires_at <= Utc::now() {
        return Err(AppError::AuthDeviceCodeExpired);
    }
    Ok(row)
}

pub async fn options(
    db: &Database,
    hmac_key: &[u8],
    actor: &str,
    code: &str,
) -> AppResult<agent::LoginOptions> {
    pending_request(db, hmac_key, code).await?;
    agent::options_for_actor(db, actor).await
}

#[allow(clippy::too_many_arguments)]
pub async fn approve_with_agent_key(
    db: &Database,
    encryption: &EncryptionKeys,
    hmac_key: &[u8],
    input: ApproveInput,
    selection: agent::Selection,
    credential_expires_at: Option<DateTime<Utc>>,
) -> AppResult<()> {
    let row = pending_request(db, hmac_key, &input.user_code).await?;
    let collection = collection_for_protocol(db, row.supports_grant_choice);
    let key_id = match &selection {
        agent::Selection::Existing { api_key_id } => {
            agent::eligible_key(db, &input.user_id, api_key_id)
                .await?
                .id
        }
        agent::Selection::New(_) => Uuid::new_v4().to_string(),
    };
    let created = matches!(selection, agent::Selection::New(_));
    let credential_id = Uuid::new_v4().to_string();
    let secret = credentials::generate_secret();
    let encrypted = encryption.encrypt(secret.as_bytes()).await?;
    let context = InitiateInput {
        client_label: row.client_label.clone(),
        client_user_agent: row.client_user_agent.clone(),
        ..Default::default()
    };
    let mut transaction = db.client().start_session().await?;
    {
        let db_owned = db.clone();
        let hmac_owned = Zeroizing::new(hmac_key.to_vec());
        let input = input.clone();
        let row = row.clone();
        let key_id = key_id.clone();
        let credential_id = credential_id.clone();
        transaction.start_transaction().and_run2(async move |session| {
        let db = &db_owned;
        let hmac_key = hmac_owned.as_slice();
        let operation: AppResult<()> = async {
            let claimed = collection.find_one_and_update(
                doc! {"_id": &row.id, "status": "pending", "expires_at": {"$gt": bson::DateTime::from_chrono(Utc::now())}},
                doc! {"$set": {"status": "approved"}},
            ).session(&mut *session).await?;
            if claimed.is_none() { return Err(current_decision_error(&collection, &row.id).await?); }
            agent::issue_selected(db, &input.user_id, &row.id, &context, row.requested_profile.as_deref(), &selection,
                credential_expires_at, &key_id, &credential_id, &secret, &mut *session).await?;
            let now = Utc::now();
            if row.expires_at <= now { return Err(AppError::AuthDeviceCodeExpired); }
            collection.update_one(doc! {"_id": &row.id}, doc! {"$set": {
                "approved_user_id": &input.user_id,
                "approved_at": bson::DateTime::from_chrono(now),
                "approver_ip_hmac": input.approver_ip.as_deref().map(|ip| hmac_hex(hmac_key, ip.as_bytes())),
                "expires_at": bson::DateTime::from_chrono(now + Duration::seconds(60)),
                "agent_key_grant": {"api_key_id": &key_id, "credential_id": &credential_id,
                    "key_was_created": created,
                    "delivery_credential_encrypted": bson::Binary {subtype: BinarySubtype::Generic, bytes: encrypted.clone()}}
            }}).session(&mut *session).await?;
            Ok(())
        }.await;
        mutations::transaction_result(operation)
    }).await.map_err(mutations::map_transaction_error)?;
    }
    audit_service::log_async(
        db.clone(),
        Some(input.user_id),
        "auth_device_code_approved".into(),
        Some(
            serde_json::json!({"request_id": row.id, "auth_kind": "agent_key", "api_key_id": key_id, "credential_id": credential_id}),
        ),
        input.approver_ip,
        input.approver_user_agent,
        None,
        None,
    );
    Ok(())
}

pub async fn poll_agent_key(
    db: &Database,
    encryption: &EncryptionKeys,
    hmac_key: &[u8],
    device_code: &str,
) -> AppResult<agent::Delivery> {
    let collection = collection_for_device_code(db, device_code);
    let row = collection
        .find_one(doc! {"device_code_hmac": hmac_hex(hmac_key, device_code.as_bytes())})
        .await?
        .ok_or(AppError::AuthDeviceCodeNotFound)?;
    if row.status != AuthDeviceCodeStatus::Approved {
        return Err(non_pending_approve_error(row.status));
    }
    if row.expires_at <= Utc::now() {
        expire(db, &row).await?;
        return Err(AppError::AuthDeviceCodeExpired);
    }
    let grant = row
        .agent_key_grant
        .as_ref()
        .ok_or(AppError::AgentKeyCredentialNotFound)?;
    // KMS and parent eligibility failures retain the encrypted delivery for retry.
    let delivery = agent::prepare_credential_delivery(
        db,
        encryption,
        Some(&grant.credential_id),
        row.approved_user_id.as_deref(),
        grant.delivery_credential_encrypted.as_deref(),
        grant.key_was_created,
    )
    .await?;
    let now = Utc::now();
    let claimed = collection.find_one_and_update(
        doc! {"_id": &row.id, "status": "approved", "expires_at": {"$gt": bson::DateTime::from_chrono(now)}},
        doc! {"$set": {"status": "delivered", "delivered_at": bson::DateTime::from_chrono(now),
            "purge_at": bson::DateTime::from_chrono(now + Duration::days(1))},
            "$unset": {"agent_key_grant.delivery_credential_encrypted": ""}},
    ).await?;
    if claimed.is_none() {
        return Err(current_decision_error(&collection, &row.id).await?);
    }
    audit_service::log_async(
        db.clone(),
        row.approved_user_id,
        "auth_device_code_delivered".into(),
        Some(
            serde_json::json!({"request_id": row.id, "auth_kind": "agent_key", "credential_id": grant.credential_id}),
        ),
        None,
        None,
        None,
        None,
    );
    Ok(delivery)
}

pub(super) async fn expire(db: &Database, row: &AuthDeviceCode) -> AppResult<()> {
    let collection = collection_for_protocol(db, row.supports_grant_choice);
    let now = Utc::now();
    let claimed = collection
        .find_one_and_update(
            doc! {"_id": &row.id, "status": {"$in": ["pending", "approved", "expired"]},
            "expires_at": {"$lte": bson::DateTime::from_chrono(now)}},
            doc! {"$set": {"status": "expired"}, "$unset": {
            "delivery_access_token_encrypted": "", "delivery_refresh_token_encrypted": "",
            "agent_key_grant.delivery_credential_encrypted": ""}},
        )
        .return_document(ReturnDocument::After)
        .await?;
    if let Some(row) = claimed {
        if let Some(id) = row.approved_session_id.as_deref() {
            token_service::revoke_session(db, id, None).await?;
        }
        if let Some(grant) = row.agent_key_grant {
            credentials::revoke(
                db,
                &grant.credential_id,
                CredentialRevokedReason::UndeliveredExpired,
            )
            .await?;
        }
        collection
            .update_one(
                doc! {"_id": &row.id, "status": "expired"},
                doc! {"$set": {"purge_at": bson::DateTime::from_chrono(now + Duration::days(1))}},
            )
            .await?;
    }
    Ok(())
}

pub async fn sweep_expired(db: &Database) -> AppResult<()> {
    for supports_grant_choice in [false, true] {
        let mut rows = collection_for_protocol(db, supports_grant_choice).find(doc! {"status": {"$in": ["pending", "approved", "expired"]},
        "expires_at": {"$lte": bson::DateTime::from_chrono(Utc::now())}, "purge_at": Bson::Null}).await?;
        while let Some(row) = rows.try_next().await? {
            if let Err(error) = expire(db, &row).await {
                tracing::error!(request_id = %row.id, error_code = error.error_code(), "auth_device expiry cleanup failed");
            }
        }
    }
    Ok(())
}
