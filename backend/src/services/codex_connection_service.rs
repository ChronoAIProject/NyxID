use std::collections::HashMap;

#[cfg(test)]
mod tests;

use chrono::Utc;
use mongodb::{
    Database,
    bson::{self, doc},
    options::ReturnDocument,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    crypto::aes::EncryptionKeys,
    errors::{AppError, AppResult},
    models::{
        provider_config::{COLLECTION_NAME as PROVIDERS, ProviderConfig},
        user::{COLLECTION_NAME as USERS, User},
        user_provider_token::{COLLECTION_NAME as TOKENS, UserProviderToken},
        user_service::{COLLECTION_NAME as SERVICES, UserService},
    },
    services::{
        api_key_mutation_service as transactions, llm_gateway_service,
        unified_key_service as unified, user_api_key_service,
    },
};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, utoipa::ToSchema)]
#[schema(as = CodexConnectionVersion)]
pub struct ConnectionVersion {
    pub id: String,
    pub state_version: i64,
}

pub async fn provider(db: &Database) -> AppResult<ProviderConfig> {
    db.collection::<ProviderConfig>(PROVIDERS)
        .find_one(doc! {"slug":"openai", "provider_type":"api_key", "is_active":true})
        .await?
        .ok_or_else(|| {
            AppError::NotFound(
                "OpenAI API-key provider is unavailable; use AI Services to authorize a provider"
                    .into(),
            )
        })
}

pub async fn current(
    db: &Database,
    user: &str,
    provider_id: &str,
) -> AppResult<Option<UserProviderToken>> {
    if db
        .collection::<UserProviderToken>(TOKENS)
        .count_documents(doc! {"user_id":user,"provider_config_id":provider_id})
        .limit(2)
        .await?
        > 1
    {
        return Err(AppError::Conflict(
            "Multiple OpenAI provider records exist; manage them in AI Services before importing"
                .into(),
        ));
    }
    Ok(db
        .collection::<UserProviderToken>(TOKENS)
        .find_one(doc! {"user_id":user, "provider_config_id":provider_id})
        .sort(doc! {"updated_at":-1})
        .await?)
}

pub fn version(token: &UserProviderToken) -> ConnectionVersion {
    ConnectionVersion {
        id: token.id.clone(),
        state_version: token.state_version,
    }
}

pub async fn connection_status(
    db: &Database,
    token: &UserProviderToken,
) -> AppResult<&'static str> {
    if token.status != "active" {
        return Ok("reconnect_required");
    }
    let Some(service_id) = token.metadata.as_ref().and_then(|m| m.get("service_id")) else {
        return Ok("saved");
    };
    let binding = match verification_binding(db, token, service_id).await {
        Ok(binding) => binding,
        Err(AppError::BadRequest(_) | AppError::Conflict(_)) => return Ok("reconnect_required"),
        Err(error) => return Err(error),
    };
    let metadata = token.metadata.as_ref();
    if metadata.and_then(|m| m.get("verification_state_version"))
        != Some(&token.state_version.to_string())
    {
        return Ok("saved");
    }
    let verified_binding = metadata
        .and_then(|m| m.get("verification_binding"))
        .and_then(|value| serde_json::from_str::<VerificationBinding>(value).ok());
    if verified_binding.as_ref() != Some(&binding) {
        return Ok("saved");
    }
    Ok(
        match metadata
            .and_then(|m| m.get("verification_status"))
            .map(String::as_str)
        {
            Some("usable") => "usable",
            Some("reconnect_required") => "reconnect_required",
            _ => "saved",
        },
    )
}

pub async fn import_api_key(
    db: &Database,
    encryption: &EncryptionKeys,
    user: &str,
    api_key: &str,
    expected: Option<&ConnectionVersion>,
) -> AppResult<UserProviderToken> {
    if !api_key.starts_with("sk-")
        || !(8..=4096).contains(&api_key.len())
        || api_key.chars().any(char::is_whitespace)
    {
        return Err(AppError::ValidationError("A supported OpenAI API key is required. ChatGPT OAuth sessions require separate authorization.".into()));
    }
    let provider = provider(db).await?;
    let (catalog, _) = llm_gateway_service::resolve_llm_service_by_slug(db, "openai").await?;
    let encrypted = encryption.encrypt(api_key.as_bytes()).await?;
    let id = Uuid::new_v4().to_string();
    let service_id = Uuid::new_v4().to_string();
    let db = db.clone();
    let user = user.to_string();
    let expected = expected.cloned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let operation: AppResult<UserProviderToken> = async {
            // Serialize two first imports before either has a token row to lock.
            let owner = db.collection::<User>(USERS).update_one(doc! {"_id":&user},
                doc! {"$inc":{"provider_import_fence":1_i64}}).session(&mut *session).await?;
            if owner.matched_count != 1 { return Err(AppError::Unauthorized("Account is unavailable".into())); }
            let tokens = db.collection::<UserProviderToken>(TOKENS);
            if tokens.count_documents(doc! {"user_id":&user,"provider_config_id":&provider.id})
                .limit(2).session(&mut *session).await? > 1 {
                return Err(AppError::Conflict("Multiple provider connections exist; manage them in AI Services before importing".into()));
            }
            let old = tokens.find_one(doc! {"user_id":&user,"provider_config_id":&provider.id})
                .sort(doc! {"updated_at":-1}).session(&mut *session).await?;
            if old.as_ref().map(version) != expected {
                return Err(AppError::Conflict("Provider connection changed; review and confirm replacement again".into()));
            }
            let now = Utc::now();
            let mut metadata = HashMap::from([
                ("credential_source".into(), "codex_api_key_file".into()),
                ("verification_status".into(), "saved".into()),
                ("service_id".into(), service_id.clone()),
            ]);
            if let Some(old) = old {
                if let Some(id) = old.metadata.as_ref().and_then(|m| m.get("service_id")) {
                    let service = db.collection::<UserService>(SERVICES).find_one(doc! {
                        "_id":id,"user_id":&user,"is_active":true,
                    }).session(&mut *session).await?;
                    if let Some(service) = service
                        && let Some(key_id) = service.api_key_id
                        && db.collection::<crate::models::user_api_key::UserApiKey>(crate::models::user_api_key::COLLECTION_NAME)
                            .find_one(doc! {"_id":key_id,"user_id":&user,"status":"active",
                                "provider_config_id":&provider.id,"connection_id":null})
                            .session(&mut *session).await?.is_some() {
                        metadata.insert("service_id".into(), id.clone());
                    }
                }
                let token = tokens.find_one_and_update(doc! {"_id":&old.id,"state_version":old.state_version},
                    doc! {"$set": {"api_key_encrypted":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted.clone()},
                        "status":"active","token_type":"api_key","metadata":bson::to_bson(&metadata).map_err(|_| AppError::Internal("Connection metadata encoding failed".into()))?,
                        "updated_at":bson::DateTime::from_chrono(now), "error_message":bson::Bson::Null,
                        "access_token_encrypted":bson::Bson::Null,"refresh_token_encrypted":bson::Bson::Null,
                        "expires_at":bson::Bson::Null,"gateway_url":bson::Bson::Null}, "$inc":{"state_version":1_i64}})
                    .return_document(ReturnDocument::After).session(&mut *session).await?
                    .ok_or_else(|| AppError::Conflict("Provider connection changed; confirm replacement again".into()))?;
                user_api_key_service::replace_provider_api_key_in_transaction(&db, session, &token).await?;
                let service = token.metadata.as_ref().and_then(|m| m.get("service_id")).expect("import service ID");
                unified::provision_imported_api_key_in_transaction(&db, session, &token, &catalog, service).await?;
                return Ok(token);
            }
            let token = UserProviderToken {id:id.clone(),user_id:user.clone(),provider_config_id:provider.id.clone(),
                connection_id:None,credential_user_id:None,token_type:"api_key".into(),access_token_encrypted:None,
                refresh_token_encrypted:None,token_scopes:None,expires_at:None,api_key_encrypted:Some(encrypted.clone()),
                status:"active".into(),state_version:1,last_refreshed_at:None,last_used_at:None,error_message:None,
                label:Some("Codex API key".into()),metadata:Some(metadata),gateway_url:None,created_at:now,updated_at:now};
            tokens.insert_one(&token).session(&mut *session).await?;
            unified::provision_imported_api_key_in_transaction(&db, session, &token, &catalog, &service_id).await?;
            Ok(token)
        }.await;
        transactions::transaction_result(operation)
    }).await.map_err(transactions::map_transaction_error)
}

pub async fn require_current(
    db: &Database,
    user: &str,
    expected: &ConnectionVersion,
) -> AppResult<UserProviderToken> {
    let provider = provider(db).await?;
    db.collection::<UserProviderToken>(TOKENS).find_one(doc! {"_id":&expected.id,"user_id":user,
        "provider_config_id":provider.id,"token_type":"api_key","metadata.credential_source":"codex_api_key_file",
        "state_version":expected.state_version,"status":"active"})
        .await?.ok_or_else(|| AppError::Conflict("Saved connection changed; refresh its status before verifying".into()))
}

pub async fn account_email(db: &Database, user: &str) -> AppResult<String> {
    Ok(db
        .collection::<User>(USERS)
        .find_one(doc! {"_id":user})
        .await?
        .ok_or_else(|| AppError::Unauthorized("Account is unavailable".into()))?
        .email)
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationBinding {
    pub key_id: String,
    pub credential_epoch: i64,
    service_version: i64,
    endpoint_id: String,
    endpoint_updated_at: String,
}

pub async fn verification_binding(
    db: &Database,
    token: &UserProviderToken,
    service_id: &str,
) -> AppResult<VerificationBinding> {
    let service = db
        .collection::<UserService>(SERVICES)
        .find_one(doc! {"_id":service_id,"user_id":&token.user_id,"is_active":true})
        .await?
        .ok_or_else(|| {
            AppError::BadRequest("Enable the saved AI service before verification".into())
        })?;
    let id = service
        .api_key_id
        .ok_or_else(|| AppError::Conflict("Saved service credential changed".into()))?;
    let key = db
        .collection::<crate::models::user_api_key::UserApiKey>(
            crate::models::user_api_key::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&id,"user_id":&token.user_id,"status":"active"})
        .await?
        .ok_or_else(|| AppError::Conflict("Saved service credential changed".into()))?;
    if key.provider_config_id.as_deref() != Some(&token.provider_config_id)
        || key.credential_encrypted != token.api_key_encrypted
        || key.credential_type != "api_key"
        || key.expires_at.is_some_and(|expiry| expiry <= Utc::now())
    {
        return Err(AppError::Conflict(
            "Saved service no longer uses this credential; reconnect before verification".into(),
        ));
    }
    let endpoint = db
        .collection::<crate::models::user_endpoint::UserEndpoint>(
            crate::models::user_endpoint::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&service.endpoint_id,"user_id":&token.user_id})
        .await?
        .ok_or_else(|| AppError::Conflict("Saved service endpoint was deleted".into()))?;
    Ok(VerificationBinding {
        key_id: id,
        credential_epoch: key.credential_epoch,
        service_version: service.state_version,
        endpoint_id: endpoint.id,
        endpoint_updated_at: endpoint.updated_at.to_rfc3339(),
    })
}

pub async fn registered_service(db: &Database, token: &UserProviderToken) -> AppResult<String> {
    require_current(db, &token.user_id, &version(token)).await?;
    let id = token
        .metadata
        .as_ref()
        .and_then(|m| m.get("service_id"))
        .ok_or_else(|| {
            AppError::BadRequest("This connection was not imported from Codex".into())
        })?;
    verification_binding(db, token, id).await?;
    Ok(id.clone())
}

pub async fn record_verification(
    db: &Database,
    user: &str,
    expected: &ConnectionVersion,
    binding: &VerificationBinding,
    status: &str,
) -> AppResult<()> {
    let binding = serde_json::to_string(binding)
        .map_err(|_| AppError::Internal("Verification metadata encoding failed".into()))?;
    let updated = db.collection::<UserProviderToken>(TOKENS).update_one(doc! {
        "_id":&expected.id,"user_id":user,"state_version":expected.state_version,"status":"active",
    }, doc! {"$set":{"metadata.verification_status":status,"metadata.verification_state_version":expected.state_version.to_string(),
        "metadata.verification_binding":binding,"metadata.verified_at":Utc::now().to_rfc3339()}}).await?;
    if updated.matched_count != 1 {
        return Err(AppError::Conflict(
            "Connection changed during verification; verify again".into(),
        ));
    }
    Ok(())
}
