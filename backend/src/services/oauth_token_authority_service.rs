//! Shared live OAuth authority for REST, MCP, introspection and exchange.
//! Permission snapshots never survive a majority-acknowledged revoke/regrant.
use bson::doc;
use chrono::Utc;
use mongodb::{
    Database,
    options::{CollectionOptions, ReadConcern, ReadPreference, SelectionCriteria, WriteConcern},
};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};

use super::consent_service;
use crate::{
    crypto::jwt::{Claims, DelegationOrigin},
    errors::{AppError, AppResult},
    models::{
        consent::Consent,
        oauth_client::OauthClient,
        oauth_delegation_authority::{COLLECTION_NAME, ConsentBinding, OauthDelegationAuthority},
        session::Session,
        user::User,
    },
};

fn denied() -> AppError {
    AppError::Unauthorized("OAuth authority changed or expired; authorize again".into())
}

async fn read<T: DeserializeOwned + Send + Sync>(
    db: &Database,
    collection: &str,
    filter: bson::Document,
) -> AppResult<T> {
    let collection = db.collection_with_options::<T>(
        collection,
        CollectionOptions::builder()
            .read_concern(ReadConcern::linearizable())
            .selection_criteria(SelectionCriteria::ReadPreference(ReadPreference::Primary))
            .build(),
    );
    tokio::time::timeout(std::time::Duration::from_millis(250), async {
        collection
            .find_one(filter)
            .max_time(std::time::Duration::from_millis(200))
            .await
    })
    .await
    .map_err(|_| denied())?
    .map_err(|_| denied())?
    .ok_or_else(denied)
}

fn fingerprint(value: &impl Serialize) -> AppResult<String> {
    let bytes = serde_json::to_vec(value).map_err(|_| denied())?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

async fn active_client(db: &Database, client_id: &str) -> AppResult<OauthClient> {
    read(
        db,
        crate::models::oauth_client::COLLECTION_NAME,
        doc! {"_id":client_id,"is_active":true},
    )
    .await
}

async fn source_session(db: &Database, user_id: &str, session_id: &str) -> AppResult<()> {
    let _: Session = read(
        db,
        crate::models::session::COLLECTION_NAME,
        super::session_authority::authenticated(
            doc! {"_id":session_id,"user_id":user_id,"revoked":false,
            "expires_at":{"$gt":bson::DateTime::now()}},
        ),
    )
    .await?;
    Ok(())
}

async fn bindings_live(db: &Database, row: &OauthDelegationAuthority) -> AppResult<()> {
    if row.consents.is_empty() || row.consents.len() > 3 {
        return Err(denied());
    }
    for binding in &row.consents {
        active_client(db, &binding.client_id).await?;
        consent_service::validate_protected_reference(
            db,
            &row.user_id,
            &binding.client_id,
            &binding.scopes,
            Some(&binding.reference),
        )
        .await
        .map_err(|_| denied())?;
    }
    if let Some(session) = &row.source_session_id {
        source_session(db, &row.user_id, session).await?;
    }
    let receiver = row
        .receiving_client_id
        .as_deref()
        .unwrap_or(&row.actor_client_id);
    let client = active_client(db, receiver).await?;
    super::token_exchange_service::validate_delegation_scope(&row.scope, &client.delegation_scopes)
        .map_err(|_| denied())?;
    Ok(())
}

pub async fn load_delegation(db: &Database, jti: &str) -> AppResult<OauthDelegationAuthority> {
    let row: OauthDelegationAuthority = read(
        db,
        COLLECTION_NAME,
        doc! {"_id":jti,"expires_at":{"$gt":bson::DateTime::now()}},
    )
    .await?;
    bindings_live(db, &row).await?;
    Ok(row)
}

async fn validate_lineage(
    db: &Database,
    user: &str,
    lineage: &crate::models::oauth_delegation_authority::OauthAuthorityLineage,
) -> AppResult<()> {
    if lineage.consents.len() > 3 {
        return Err(denied());
    }
    for binding in &lineage.consents {
        active_client(db, &binding.client_id).await?;
        consent_service::validate_protected_reference(
            db,
            user,
            &binding.client_id,
            &binding.scopes,
            Some(&binding.reference),
        )
        .await
        .map_err(|_| denied())?;
    }
    if let Some(session) = &lineage.source_session_id {
        source_session(db, user, session).await?;
    }
    if let Some(jti) = &lineage.delegation_jti {
        let row = load_delegation(db, jti).await?;
        if row.user_id != user {
            return Err(denied());
        }
    }
    if lineage.consents.is_empty()
        && lineage.source_session_id.is_none()
        && lineage.delegation_jti.is_none()
    {
        return Err(denied());
    }
    Ok(())
}

/// Called only after signature/type validation. Service-issued delegation has
/// its own signed origin and retains the existing service-policy contract.
pub async fn validate_claims(db: &Database, claims: &Claims) -> AppResult<()> {
    if claims.sa == Some(true) || claims.relay == Some(true) {
        return Ok(());
    }
    let _: User = read(
        db,
        crate::models::user::COLLECTION_NAME,
        doc! {"_id":&claims.sub,"is_active":true,"platform_personal_deleting":{"$ne":true}},
    )
    .await?;
    if let Some(actor) = &claims.act {
        match claims.delegation_origin {
            Some(DelegationOrigin::Service) if claims.client_id.is_none() => {
                if let Some(lineage) = &claims.oauth_lineage {
                    validate_lineage(db, &claims.sub, lineage).await?;
                }
                return Ok(());
            }
            // Legacy delegation has no authenticated origin discriminator.
            // It cannot fall back to service authority when a client is deleted.
            Some(DelegationOrigin::Oauth) => {}
            _ => return Err(denied()),
        }
        let row = load_delegation(db, &claims.jti).await?;
        if row.user_id != claims.sub
            || row.actor_client_id != actor.sub
            || row.receiving_client_id != claims.client_id
            || row.scope != claims.scope
            || row.claims_fingerprint != fingerprint(claims)?
        {
            return Err(denied());
        }
    } else if let Some(client) = claims.client_id.as_deref() {
        active_client(db, client).await?;
        consent_service::validate_protected_reference(
            db,
            &claims.sub,
            client,
            &claims.scope,
            claims.protected_consent.as_ref(),
        )
        .await
        .map_err(|_| denied())?;
    } else if let Some(session) = claims.sid.as_deref() {
        source_session(db, &claims.sub, session).await?;
    }
    Ok(())
}

/// Capture references only during an already-authorized new exchange. Renewal
/// passes the retained original bindings instead of consulting current consent.
pub async fn exchange_bindings(
    db: &Database,
    source: &Claims,
    receiver: &str,
) -> AppResult<Vec<ConsentBinding>> {
    validate_claims(db, source).await?;
    let mut bindings = Vec::new();
    if let Some(client) = &source.client_id {
        bindings.push(ConsentBinding {
            client_id: client.clone(),
            scopes: source.scope.clone(),
            reference: source.protected_consent.clone().ok_or_else(denied)?,
        });
    }
    let consent = consent_service::check_consent(db, &source.sub, receiver, "openid")
        .await?
        .ok_or_else(denied)?;
    let reference = consent_service::ensure_issuance_reference(db, &consent).await?;
    if let Some(binding) = bindings
        .iter_mut()
        .find(|binding| binding.client_id == receiver)
    {
        // This exact same source revision must also include openid.
        consent_service::validate_protected_reference(
            db,
            &source.sub,
            receiver,
            "openid",
            Some(&binding.reference),
        )
        .await?;
        if !binding
            .scopes
            .split_whitespace()
            .any(|scope| scope == "openid")
        {
            binding.scopes.push_str(" openid");
        }
    } else {
        bindings.push(ConsentBinding {
            client_id: receiver.into(),
            scopes: "openid".into(),
            reference,
        });
    }
    Ok(bindings)
}

pub async fn persist(
    db: &Database,
    claims: &Claims,
    bindings: Vec<ConsentBinding>,
    source_session_id: Option<String>,
) -> AppResult<()> {
    let row = OauthDelegationAuthority {
        id: claims.jti.clone(),
        user_id: claims.sub.clone(),
        actor_client_id: claims.act.as_ref().ok_or_else(denied)?.sub.clone(),
        receiving_client_id: claims.client_id.clone(),
        scope: claims.scope.clone(),
        claims_fingerprint: fingerprint(claims)?,
        consents: bindings,
        source_session_id,
        expires_at: chrono::DateTime::from_timestamp(claims.exp, 0).ok_or_else(denied)?,
        created_at: Utc::now(),
    };
    bindings_live(db, &row).await?;
    let mut transaction = db.client().start_session().await?;
    transaction
        .start_transaction()
        .write_concern(WriteConcern::majority())
        .await?;
    let result: AppResult<()> = async {
        let live = db.collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
            .update_one(doc! {"_id":&row.user_id,"is_active":true,"platform_personal_deleting":{"$ne":true}},
                doc! {"$set":{"oauth_issuance_fence":uuid::Uuid::new_v4().to_string()}})
            .session(&mut transaction).await?;
        if live.matched_count != 1 { return Err(denied()); }
        for binding in &row.consents {
            let client = db.collection::<OauthClient>(crate::models::oauth_client::COLLECTION_NAME)
                .update_one(doc! {"_id":&binding.client_id,"is_active":true},
                    doc! {"$set":{"oauth_issuance_fence":uuid::Uuid::new_v4().to_string()}})
                .session(&mut transaction).await?;
            if client.matched_count != 1 { return Err(denied()); }
            let consent = db.collection::<Consent>(crate::models::consent::COLLECTION_NAME)
                .update_one(doc! {"_id":&binding.reference.consent_id,"revision":&binding.reference.consent_revision,
                    "user_id":&row.user_id,"client_id":&binding.client_id},
                    doc! {"$set":{"issuance_fence":uuid::Uuid::new_v4().to_string()}})
                .session(&mut transaction).await?;
            if consent.matched_count != 1 { return Err(denied()); }
        }
        if let Some(session) = &row.source_session_id {
            let live = db.collection::<Session>(crate::models::session::COLLECTION_NAME)
                .update_one(super::session_authority::authenticated(doc! {"_id":session,"user_id":&row.user_id,
                    "revoked":false,"expires_at":{"$gt":bson::DateTime::now()}}),
                    doc! {"$set":{"oauth_issuance_fence":uuid::Uuid::new_v4().to_string()}})
                .session(&mut transaction).await?;
            if live.matched_count != 1 { return Err(denied()); }
        }
        db.collection::<OauthDelegationAuthority>(COLLECTION_NAME).insert_one(&row)
            .session(&mut transaction).await?;
        Ok(())
    }.await;
    if let Err(error) = result {
        transaction.abort_transaction().await?;
        return Err(error);
    }
    super::transaction_service::commit(&mut transaction).await
}

#[cfg(test)]
mod tests;
