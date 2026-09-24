//! Service-bound RFC 7662 evidence for ordinary agent API keys.
//! The confidential resource server must be registered on the target catalog
//! service. Neither a public OAuth client nor a key's owner identity alone
//! authorizes a downstream service.

use chrono::{DateTime, Utc};
use mongodb::{Database, bson::doc};

use crate::{
    errors::{AppError, AppResult},
    models::{
        api_key::ApiKeyPurpose,
        api_key_credential::{ApiKeyCredential, COLLECTION_NAME as CREDENTIALS},
        downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService},
        oauth_client::OauthClient,
        user::{COLLECTION_NAME as USERS, User},
        user_service::{COLLECTION_NAME as CONNECTIONS, UserService},
    },
    services::{key_service, rbac_helpers},
};

pub struct Evidence {
    pub subject: String,
    pub service_id: String,
    pub key_id: String,
    pub scope: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub permissions: Vec<String>,
}

fn refused() -> AppError {
    AppError::Unauthorized("Agent key is not active for this resource server".into())
}

pub async fn introspect(
    db: &Database,
    client: &OauthClient,
    service_id: &str,
    raw_key: &str,
) -> AppResult<Evidence> {
    if client.client_type != "confidential" || !client.is_active {
        return Err(refused());
    }
    let service = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id": service_id, "is_active": true})
        .await?
        .ok_or_else(refused)?;
    if !service
        .developer_app_ids
        .as_ref()
        .is_some_and(|ids| ids.contains(&client.id))
    {
        return Err(refused());
    }
    let (subject, key, child_id) = key_service::validate_api_key(db, raw_key).await?;
    if key.purpose != ApiKeyPurpose::General || !key.scopes.split_whitespace().any(|s| s == "proxy")
    {
        return Err(refused());
    }
    db.collection::<User>(USERS)
        .find_one(doc! {"_id": &subject, "is_active": true})
        .await?
        .ok_or_else(refused)?;
    let mut filter = doc! {"user_id": &subject, "catalog_service_id": service_id, "is_active": true, "deleted_at": null};
    if !key.allow_all_services {
        let allowed = key_service::effective_allowed_service_ids(db, &key).await?;
        filter.insert("_id", doc! {"$in": allowed});
    }
    // An active, owner-bound service connection must exist. Introspection never
    // provisions a connection or widens an agent's allowlist.
    db.collection::<UserService>(CONNECTIONS)
        .find_one(filter)
        .await?
        .ok_or_else(refused)?;
    let mut expires_at = key.expires_at;
    if let Some(child_id) = child_id {
        let child = db.collection::<ApiKeyCredential>(CREDENTIALS)
            .find_one(doc! {"_id": child_id, "api_key_id": &key.id, "user_id": &subject, "is_active": true})
            .await?.ok_or_else(refused)?;
        expires_at = earliest_expiry(expires_at, child.expires_at);
    }
    if expires_at.is_some_and(|expiry| expiry <= Utc::now()) {
        return Err(refused());
    }
    let rbac = rbac_helpers::resolve_user_rbac(db, &subject).await?;
    Ok(Evidence {
        subject,
        service_id: service.id,
        key_id: key.id,
        scope: key.scopes,
        expires_at,
        created_at: key.created_at,
        permissions: rbac.permissions,
    })
}

fn earliest_expiry(
    parent: Option<DateTime<Utc>>,
    child: Option<DateTime<Utc>>,
) -> Option<DateTime<Utc>> {
    match (parent, child) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
mod tests;
