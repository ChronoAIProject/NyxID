//! Aurinko's named confidential-client account-code protocol. No generic PKCE bypass.
use bson::{Document, doc};
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::{ClientSession, Database, options::ReturnDocument};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use uuid::Uuid;
use zeroize::Zeroizing;

use super::{
    api_key_mutation_service as transactions, org_service, unified_key_service as unified,
};
use crate::{
    crypto::aes::EncryptionKeys,
    errors::{AppError, AppResult},
    models::{
        oauth_state::{AurinkoOAuthBinding, COLLECTION_NAME as STATES, OAuthState},
        provider_config::{COLLECTION_NAME as PROVIDERS, ProviderConfig},
        user_api_key::{AurinkoAccount, COLLECTION_NAME as KEYS, UserApiKey},
        user_service::{COLLECTION_NAME as SERVICES, UserService},
    },
};

pub const PROTOCOL: &str = "aurinko_account_code";
pub const CALLBACK_PATH: &str = "/api/v1/providers/aurinko/mailboxes/callback";
pub const SCOPES: &[&str] = &["Mail.Read", "Mail.Send", "Mail.Drafts"];
pub const ORIGIN: &str = "https://api.aurinko.io";

pub async fn validate_connection_route(
    db: &Database,
    service: &UserService,
    key: &UserApiKey,
) -> AppResult<()> {
    if !is_managed_key(db, key).await? {
        return Ok(());
    }
    let endpoint = db
        .collection::<crate::models::user_endpoint::UserEndpoint>("user_endpoints")
        .find_one(doc! {"_id":&service.endpoint_id,"user_id":&key.user_id})
        .await?
        .ok_or_else(changed)?;
    validate_managed_route_snapshot(service, key, &endpoint)
}

/// Validate the actual endpoint that will receive the credential, including when
/// it was loaded before a concurrent endpoint update.
pub(crate) async fn validate_connection_route_snapshot(
    db: &Database,
    service: &UserService,
    key: &UserApiKey,
    endpoint: &crate::models::user_endpoint::UserEndpoint,
) -> AppResult<()> {
    if !is_managed_key(db, key).await? {
        return Ok(());
    }
    validate_managed_route_snapshot(service, key, endpoint)
}

fn validate_managed_route_snapshot(
    service: &UserService,
    key: &UserApiKey,
    endpoint: &crate::models::user_endpoint::UserEndpoint,
) -> AppResult<()> {
    if service.user_id != key.user_id
        || service.node_id.is_some()
        || service.auth_method != "bearer"
        || service.auth_key_name != "Authorization"
        || service.credential_binding.as_deref() == Some("platform")
        || endpoint.id != service.endpoint_id
        || endpoint.user_id != key.user_id
        || endpoint.url.trim_end_matches('/') != ORIGIN
    {
        return Err(changed());
    }
    Ok(())
}

pub async fn key_for_connection(
    db: &Database,
    owner: &str,
    connection: &str,
) -> AppResult<UserApiKey> {
    db.collection::<UserApiKey>(KEYS)
        .find_one(doc! {"user_id":owner,"connection_id":connection})
        .await?
        .ok_or_else(invalid)
}

pub async fn list_connections(
    db: &Database,
    actor: &str,
    owner: &str,
) -> AppResult<Vec<(UserService, UserApiKey)>> {
    let access = org_service::resolve_owner_access(db, actor, owner).await?;
    if !access.can_read() {
        return Err(AppError::Forbidden(
            "Mailbox owner access is required".into(),
        ));
    }
    let Some(provider) = db
        .collection::<ProviderConfig>(PROVIDERS)
        .find_one(doc! {"slug":"aurinko"})
        .await?
    else {
        return Ok(vec![]);
    };
    let keys: Vec<UserApiKey> = db.collection::<UserApiKey>(KEYS).find(doc! {"user_id":owner,"provider_config_id":provider.id,"credential_type":"oauth2","credential_source":"platform"}).await?.try_collect().await?;
    let ids: Vec<_> = keys.iter().map(|k| k.id.as_str()).collect();
    let services: Vec<UserService> = db
        .collection::<UserService>(SERVICES)
        .find(doc! {"user_id":owner,"api_key_id":{"$in":ids}})
        .await?
        .try_collect()
        .await?;
    Ok(services
        .into_iter()
        .filter(|s| access.allows_resource(&s.id) && (!s.admin_only || access.can_write()))
        .filter_map(|s| {
            let key = keys
                .iter()
                .find(|k| s.api_key_id.as_deref() == Some(&k.id))?
                .clone();
            Some((s, key))
        })
        .collect())
}

/// Resolve hosted return routing only after the dedicated browser binding passes.
pub async fn bound_connect_link(
    db: &Database,
    id: &str,
    actor: &str,
    session: &str,
    nonce: &str,
) -> AppResult<Option<(String, String)>> {
    let state = db.collection::<OAuthState>(STATES).find_one(doc! {"_id":id,"user_id":actor,"aurinko.session_id":session,"consumed":false,"expires_at":{"$gt":bson::DateTime::now()}}).await?.ok_or_else(invalid)?;
    let binding = state.aurinko.as_ref().ok_or_else(invalid)?;
    let hash = hex::encode(Sha256::digest(nonce.as_bytes()));
    if !bool::from(hash.as_bytes().ct_eq(binding.browser_nonce_hash.as_bytes())) {
        return Err(invalid());
    }
    Ok(state.connect_link_id.zip(state.target_user_id))
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub enum MailProvider {
    Google,
    Office365,
    Zoho,
    #[serde(rename = "IMAP")]
    Imap,
    #[serde(rename = "EWS")]
    Ews,
    #[serde(rename = "iCloud")]
    ICloud,
}
impl MailProvider {
    pub fn service_type(self) -> &'static str {
        match self {
            Self::Google => "Google",
            Self::Office365 => "Office365",
            Self::Zoho => "Zoho",
            Self::Imap => "IMAP",
            Self::Ews => "EWS",
            Self::ICloud => "iCloud",
        }
    }
}

/// Derived for both old and newly seeded rows; never changes manual credentials or admin settings.
pub fn available(provider: &ProviderConfig) -> bool {
    provider.slug == "aurinko"
        && provider.provider_type == "api_key"
        && provider.is_active
        && !provider.requires_gateway_url
        && provider.credential_mode == "admin"
        && provider
            .client_id_encrypted
            .as_ref()
            .is_some_and(|v| !v.is_empty())
        && provider
            .client_secret_encrypted
            .as_ref()
            .is_some_and(|v| !v.is_empty())
}

fn invalid() -> AppError {
    AppError::ValidationError("Mailbox authorization is invalid or expired; connect again".into())
}
fn changed() -> AppError {
    AppError::Conflict("Mailbox connection, session, owner access, or platform configuration changed; connect again".into())
}
fn upstream() -> AppError {
    AppError::ChannelPlatformError(
        "Aurinko mailbox authorization failed; check provider setup and try again".into(),
    )
}
fn fingerprint(provider: &ProviderConfig) -> String {
    let mut hash = Sha256::new();
    hash.update(PROTOCOL);
    hash.update(&provider.id);
    for bytes in [
        &provider.client_id_encrypted,
        &provider.client_secret_encrypted,
    ] {
        let bytes = bytes.as_deref().unwrap_or_default();
        hash.update((bytes.len() as u64).to_be_bytes());
        hash.update(bytes);
    }
    hash.update(provider.updated_at.timestamp_millis().to_be_bytes());
    hex::encode(hash.finalize())
}
async fn provider(db: &Database) -> AppResult<ProviderConfig> {
    db.collection::<ProviderConfig>(PROVIDERS)
        .find_one(doc! {"slug":"aurinko"})
        .await?
        .filter(available)
        .ok_or_else(|| {
            AppError::ValidationError("Managed Aurinko mailboxes are not configured".into())
        })
}

pub fn callback_url(base_url: &str) -> AppResult<String> {
    let base = url::Url::parse(base_url).map_err(|_| invalid())?;
    if base.scheme() != "https"
        || !base.username().is_empty()
        || base.password().is_some()
        || base.query().is_some()
        || base.fragment().is_some()
        || !matches!(base.path(), "" | "/")
    {
        return Err(AppError::ValidationError(
            "Managed Aurinko requires an HTTPS public BASE_URL with no path".into(),
        ));
    }
    Ok(format!(
        "{}{}",
        base.origin().ascii_serialization(),
        CALLBACK_PATH
    ))
}

async fn authorize_owner(
    db: &Database,
    actor: &str,
    owner: &str,
    key_id: Option<&str>,
) -> AppResult<()> {
    let users = db.collection::<Document>("users");
    if users.find_one(doc! {"_id":actor,"is_active":true,"user_type":{"$in":["person",null]}}).await?.is_none()
        || users.find_one(doc! {"_id":owner,"is_active":true,"user_type":if actor==owner {bson::Bson::Document(doc!{"$in":["person",null]})} else {bson::Bson::String("org".into())}}).await?.is_none() { return Err(changed()); }
    let access = org_service::resolve_owner_access(db, actor, owner).await?;
    let service_ids: Vec<String> = if let Some(id) = key_id {
        db.collection::<UserService>(SERVICES)
            .find(doc! {"user_id":owner, "api_key_id":id, "is_active":true})
            .await?
            .try_collect::<Vec<_>>()
            .await?
            .into_iter()
            .map(|s| s.id)
            .collect()
    } else {
        vec![]
    };
    if !access.can_write() || !access.allows_any_resource(&service_ids) {
        return Err(AppError::Forbidden(
            "Mailbox owner administration is required".into(),
        ));
    }
    Ok(())
}

pub struct StartResult {
    pub key_id: String,
    pub service_id: String,
    pub state_id: String,
    pub authorization_url: String,
    pub browser_nonce: Zeroizing<String>,
}

#[allow(clippy::too_many_arguments)]
pub async fn start(
    db: &Database,
    keys: &EncryptionKeys,
    base_url: &str,
    actor: &str,
    session_id: &str,
    owner: &str,
    label: &str,
    mail_provider: MailProvider,
    key_id: Option<&str>,
    connect_link_id: Option<&str>,
    custom_slug: Option<&str>,
) -> AppResult<StartResult> {
    if label.is_empty() || label.len() > 200 {
        return Err(AppError::ValidationError(
            "Mailbox label must be 1 to 200 characters".into(),
        ));
    }
    let callback = callback_url(base_url)?;
    let provider = provider(db).await?;
    let (client_id, _client_secret) = app_credentials(keys, &provider).await?;
    authorize_owner(db, actor, owner, key_id).await?;
    require_session(db, actor, session_id).await?;
    let key = if let Some(id) = key_id {
        db.collection::<UserApiKey>(KEYS)
            .find_one(doc! {"_id":id, "user_id":owner})
            .await?
            .ok_or_else(invalid)?
    } else {
        unified::create_key(
            db,
            keys,
            owner,
            actor,
            Some("api-aurinko"),
            None,
            "",
            label,
            custom_slug,
            None,
            None,
            None,
            None,
            None,
            unified::OpenApiSpecUrlInput::Inherit,
            None,
            false,
            unified::OauthClientCredentialsInput::None,
            false,
        )
        .await?
        .api_key
        .ok_or_else(invalid)?
    };
    let attempt_nonce = Uuid::new_v4().to_string();
    let state_id = format!("1cc_{attempt_nonce}");
    let result = async {
    if key.provider_config_id.as_deref() != Some(&provider.id)
        || key.credential_type != "oauth2"
        || key.credential_source.as_deref() != Some("platform")
        || key.connection_id.is_none()
        || key.status == "revoked"
        || key.user_oauth_client_id_encrypted.is_some()
    {
        return Err(invalid());
    }
    if key
        .aurinko_account
        .as_ref()
        .is_some_and(|a| a.service_type != mail_provider.service_type())
    {
        return Err(AppError::ValidationError(
            "Reconnect using the same mailbox provider".into(),
        ));
    }
    let service = db
        .collection::<UserService>(SERVICES)
        .find_one(doc! {"user_id":owner, "api_key_id":&key.id, "is_active":true})
        .await?
        .ok_or_else(invalid)?;
    validate_connection_route(db,&service,&key).await?;
    let nonce = Zeroizing::new(hex::encode(rand::random::<[u8; 32]>()));
    let binding = AurinkoOAuthBinding {
        session_id: session_id.into(),
        browser_nonce_hash: hex::encode(Sha256::digest(nonce.as_bytes())),
        service_type: mail_provider.service_type().into(),
        config_fingerprint: fingerprint(&provider),
        key_id: key.id.clone(),
        credential_epoch: key.credential_epoch,
        account_id: key.aurinko_account.as_ref().map(|a| a.account_id.clone()),
    };
    let state = OAuthState {
        id: state_id.clone(),
        user_id: actor.into(),
        provider_config_id: provider.id.clone(),
        code_verifier: None,
        aurinko: Some(binding),
        device_code_encrypted: None,
        user_code_encrypted: None,
        poll_interval: None,
        target_user_id: Some(owner.into()),
        credential_user_id: None,
        connection_id: key.connection_id.clone(),
        connect_link_id: connect_link_id.map(str::to_owned),
        redirect_path: Some("/keys".into()),
        flow_kind: Some("cc".into()),
        attempt_nonce: Some(attempt_nonce),
        consumed: false,
        created_at: Utc::now(),
        expires_at: Utc::now() + Duration::minutes(10),
    };
    let mut session = db.client().start_session().await?;
    let state_for_txn = state.clone();
    let db_txn = db.clone();
    session.start_transaction().and_run((db_txn, state_for_txn, key.clone()), |session, (db_txn, state_for_txn, key)| Box::pin(async move {
        let operation:AppResult<()> = async {
            let owner=state_for_txn.target_user_id.as_deref().ok_or_else(invalid)?;
            fence_authority(db_txn, session, state_for_txn).await?;
            let result = db_txn.collection::<UserApiKey>(KEYS).update_one(doc! {"_id":&key.id,"user_id":owner,"credential_epoch":key.credential_epoch,"status":{"$ne":"revoked"}},
                doc! {"$set":{"oauth_attempt_nonce":&state_for_txn.id}}).session(&mut *session).await?;
            if result.matched_count != 1 { return Err(changed()); }
            db_txn.collection::<OAuthState>(STATES).insert_one(&*state_for_txn).session(&mut *session).await?;
            Ok(())
        }.await;
        transactions::transaction_result(operation)
    })).await.map_err(transactions::map_transaction_error)?;
    let authorization_url = authorization_url(
        &client_id,
        &callback,
        &state_id,
        mail_provider.service_type(),
        state.aurinko.as_ref().and_then(|b| b.account_id.as_deref()),
    )?;
    Ok(StartResult {
        key_id: state.aurinko.as_ref().expect("bound").key_id.clone(),
        service_id: service.id,
        state_id: state_id.clone(),
        authorization_url,
        browser_nonce: nonce,
    })
    }.await;
    if result.is_err() && key_id.is_none() {
        cleanup_unstarted(db, &key, &state_id).await?;
    }
    result
}

/// A start that never returned an authorization URL can remove only its original
/// empty generation. A completed or adopted connection never matches this CAS.
async fn cleanup_unstarted(db: &Database, key: &UserApiKey, attempt: &str) -> AppResult<()> {
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run((db.clone(),key.clone(),attempt.to_owned()),|session,(db,key,attempt)|Box::pin(async move {
        let result:AppResult<()> = async {
            let removed=db.collection::<UserApiKey>(KEYS).find_one_and_delete(doc! {"_id":&key.id,"user_id":&key.user_id,"credential_epoch":key.credential_epoch,"status":{"$in":["pending_auth","failed"]},"access_token_encrypted":null,"aurinko_account":null,"oauth_attempt_nonce":{"$in":[attempt.as_str(),null]}})
                .session(&mut *session).await?;
            if removed.is_some() {
                let mut cursor=db.collection::<UserService>(SERVICES).find(doc! {"user_id":&key.user_id,"api_key_id":&key.id}).session(&mut *session).await?;
                let mut services=Vec::new();
                while let Some(service)=cursor.next(&mut *session).await.transpose()? {services.push(service);}
                for service in services {
                    db.collection::<Document>(SERVICES).delete_one(doc! {"_id":&service.id}).session(&mut *session).await?;
                    db.collection::<Document>("user_endpoints").delete_one(doc! {"_id":&service.endpoint_id,"user_id":&key.user_id}).session(&mut *session).await?;
                }
                db.collection::<Document>(STATES).delete_many(doc! {"aurinko.key_id":&key.id}).session(&mut *session).await?;
            }
            Ok(())
        }.await;
        transactions::transaction_result(result)
    })).await.map_err(transactions::map_transaction_error)
}

fn authorization_url(
    client_id: &str,
    callback: &str,
    state: &str,
    service_type: &str,
    account_id: Option<&str>,
) -> AppResult<String> {
    let mut url = url::Url::parse(&format!("{ORIGIN}/v1/auth/authorize")).map_err(|_| invalid())?;
    url.query_pairs_mut().extend_pairs([
        ("clientId", client_id),
        ("serviceType", service_type),
        ("scopes", &SCOPES.join(" ")),
        ("responseType", "code"),
        ("returnUrl", callback),
        ("state", state),
    ]);
    if let Some(account) = account_id {
        url.query_pairs_mut().append_pair("accountId", account);
    }
    Ok(url.into())
}
async fn require_session(db: &Database, actor: &str, session_id: &str) -> AppResult<()> {
    if db.collection::<Document>("sessions").find_one(doc! {"_id":session_id,"user_id":actor,"revoked":false,"expires_at":{"$gt":bson::DateTime::now()}}).await?.is_none() {
        return Err(changed());
    }
    Ok(())
}

/// Write the authority rows in the same transaction as the credential to conflict
/// with logout, owner disable, membership/scope removal, and application rotation.
async fn fence_authority(
    db: &Database,
    session: &mut ClientSession,
    state: &OAuthState,
) -> AppResult<()> {
    let binding = state.aurinko.as_ref().ok_or_else(invalid)?;
    let owner = state.target_user_id.as_deref().ok_or_else(invalid)?;
    let now = bson::DateTime::now();
    let write = doc! {"$inc":{"aurinko_authorization_fence":1_i64}};
    for (id, kind) in [
        (&*state.user_id, "person"),
        (
            owner,
            if owner == state.user_id {
                "person"
            } else {
                "org"
            },
        ),
    ] {
        let type_filter = if kind == "person" {
            doc! {"$in":["person",null]}
        } else {
            doc! {"$eq":"org"}
        };
        let result = db
            .collection::<Document>("users")
            .update_one(
                doc! {"_id":id,"is_active":true,"user_type":type_filter},
                write.clone(),
            )
            .session(&mut *session)
            .await?;
        if result.matched_count != 1 {
            return Err(changed());
        }
    }
    let result=db.collection::<Document>("sessions").update_one(doc! {"_id":&binding.session_id,"user_id":&state.user_id,"revoked":false,"expires_at":{"$gt":now}},write.clone()).session(&mut *session).await?;
    if result.matched_count != 1 {
        return Err(changed());
    }
    let current = db
        .collection::<ProviderConfig>(PROVIDERS)
        .find_one(doc! {"_id":&state.provider_config_id})
        .session(&mut *session)
        .await?
        .ok_or_else(changed)?;
    if !available(&current) || fingerprint(&current) != binding.config_fingerprint {
        return Err(changed());
    }
    db.collection::<Document>(PROVIDERS)
        .update_one(doc! {"_id":&current.id}, write.clone())
        .session(&mut *session)
        .await?;
    let mut cursor = db
        .collection::<UserService>(SERVICES)
        .find(doc! {"user_id":owner,"api_key_id":&binding.key_id,"is_active":true})
        .session(&mut *session)
        .await?;
    let mut services = Vec::new();
    while let Some(service) = cursor.next(&mut *session).await.transpose()? {
        services.push(service);
    }
    if services.is_empty() {
        return Err(changed());
    }
    if let Some(link_id) = &state.connect_link_id {
        let ids: Vec<_> = services.iter().map(|s| s.id.as_str()).collect();
        let result = db
            .collection::<Document>("connect_links")
            .update_one(
                doc! {"_id":link_id,"user_id":owner,"status":"pending","completion_claim_id":null,
                "completed_user_service_id":{"$in":ids},"expires_at":{"$gt":now}},
                write.clone(),
            )
            .session(&mut *session)
            .await?;
        if result.matched_count != 1 {
            return Err(changed());
        }
    }
    if owner != state.user_id {
        use crate::models::{
            org_membership::{MemberScopeSource, OrgMembership},
            org_role_scope::OrgRoleScope,
        };
        let membership=db.collection::<OrgMembership>("org_memberships").find_one(doc! {"org_user_id":owner,"member_user_id":&state.user_id,"role":"admin","revoked_at":null}).session(&mut *session).await?.ok_or_else(changed)?;
        let allowed = match membership.scope_source {
            MemberScopeSource::Override => membership.allowed_service_ids,
            MemberScopeSource::Inherit => {
                // Materialize the default null scope so concurrent first restriction
                // shares the unique (org,role) row and participates in this fence.
                let scope=db.collection::<OrgRoleScope>("org_role_scopes").find_one_and_update(doc! {"org_user_id":owner,"role":"admin"},doc! {"$inc":{"aurinko_authorization_fence":1_i64},"$setOnInsert":{"_id":Uuid::new_v4().to_string(),"allowed_service_ids":null,"updated_by":&state.user_id,"updated_at":now}})
                    .upsert(true).return_document(ReturnDocument::After).session(&mut *session).await?.ok_or_else(changed)?;
                scope.allowed_service_ids
            }
        };
        if allowed
            .as_ref()
            .is_some_and(|ids| !services.iter().any(|s| ids.contains(&s.id)))
        {
            return Err(changed());
        }
        db.collection::<Document>("org_memberships")
            .update_one(doc! {"_id":membership.id}, write.clone())
            .session(&mut *session)
            .await?;
    }
    for service in services {
        if service.node_id.is_some()
            || service.auth_method != "bearer"
            || service.auth_key_name != "Authorization"
            || service.credential_binding.as_deref() == Some("platform")
        {
            return Err(changed());
        }
        let endpoint = db.collection::<Document>("user_endpoints").update_one(doc! {"_id":&service.endpoint_id,"user_id":owner,"url":{"$in":[ORIGIN,format!("{ORIGIN}/")]}},write.clone()).session(&mut *session).await?;
        if endpoint.matched_count != 1 {
            return Err(changed());
        }
        db.collection::<Document>(SERVICES)
            .update_one(doc! {"_id":service.id,"is_active":true}, write.clone())
            .session(&mut *session)
            .await?;
    }
    Ok(())
}

pub struct CallbackInput<'a> {
    pub state_id: &'a str,
    pub browser_nonce: &'a str,
    pub actor: &'a str,
    pub session_id: &'a str,
    pub code: Option<&'a str>,
    pub error: Option<&'a str>,
    pub status: Option<&'a str>,
}

pub async fn complete(
    db: &Database,
    keys: &EncryptionKeys,
    input: CallbackInput<'_>,
) -> AppResult<()> {
    let state = db
        .collection::<OAuthState>(STATES)
        .find_one(doc! {"_id":input.state_id})
        .await?
        .ok_or_else(invalid)?;
    let binding = state.aurinko.as_ref().ok_or_else(invalid)?;
    super::channel_retry_ingress::with_connection(
        db,
        &binding.key_id,
        complete_with_client(db, keys, input, &AurinkoClient::default()),
    )
    .await
}
async fn complete_with_client(
    db: &Database,
    keys: &EncryptionKeys,
    input: CallbackInput<'_>,
    client: &AurinkoClient,
) -> AppResult<()> {
    let state = db
        .collection::<OAuthState>(STATES)
        .find_one(
            doc! {"_id":input.state_id,"consumed":false,"expires_at":{"$gt":bson::DateTime::now()}},
        )
        .await?
        .ok_or_else(invalid)?;
    let binding = state.aurinko.as_ref().ok_or_else(invalid)?;
    let hash = hex::encode(Sha256::digest(input.browser_nonce.as_bytes()));
    if state.user_id != input.actor
        || binding.session_id != input.session_id
        || !bool::from(hash.as_bytes().ct_eq(binding.browser_nonce_hash.as_bytes()))
    {
        return Err(invalid());
    }
    require_session(db, input.actor, input.session_id).await?;
    let claimed = db
        .collection::<OAuthState>(STATES)
        .update_one(
            doc! {"_id":&state.id,"consumed":false,"expires_at":{"$gt":bson::DateTime::now()}},
            doc! {"$set":{"consumed":true}},
        )
        .await?;
    if claimed.matched_count != 1 {
        return Err(invalid());
    }
    let result=async {
        if input.error.is_some() || input.status.is_some_and(|s|s!="success") {
            return Err(AppError::ValidationError("Mailbox authorization was cancelled or declined".into()));
        }
        let code=input.code.filter(|s|!s.is_empty() && s.len()<=2048 && !s.chars().any(char::is_control)).ok_or_else(invalid)?;
        let provider=provider(db).await?;
        if provider.id!=state.provider_config_id || fingerprint(&provider)!=binding.config_fingerprint {return Err(changed());}
        let owner=state.target_user_id.as_deref().ok_or_else(invalid)?;
        authorize_owner(db,&state.user_id,owner,Some(&binding.key_id)).await?;
        let live = require_live_connection(db, owner, &binding.key_id, false).await?;
        if live.oauth_attempt_nonce.as_deref()!=Some(&state.id) || live.credential_epoch!=binding.credential_epoch || live.connection_id!=state.connection_id {return Err(changed());}
        if let Some(link_id)=&state.connect_link_id
            && db.collection::<Document>("connect_links").find_one(doc! {"_id":link_id,"user_id":owner,"status":"pending","expires_at":{"$gt":bson::DateTime::now()}}).await?.is_none() {
            return Err(changed());
        }
        let (client_id, client_secret)=app_credentials(keys,&provider).await?;
        let (token,account_id)=client.exchange(code,&client_id,&client_secret).await?;
        let (mut account,scopes)=client.account(&token,&account_id,&binding.service_type,SCOPES).await?;
        if binding.account_id.as_ref().is_some_and(|old|old!=&account.account_id) {return Err(AppError::ValidationError("Reconnect the same mailbox account".into()));}
        account.application_id_hash=hex::encode(Sha256::digest(client_id.as_bytes()));
        let encrypted=keys.encrypt(token.as_bytes()).await?;
        let db=db.clone(); let state=state.clone();
        let mut session=db.client().start_session().await?;
        session.start_transaction().and_run((db.clone(), state, encrypted, account, scopes), |session, (db, state, encrypted, account, scopes)| Box::pin(async move {
            let operation:AppResult<()> = async {
                fence_authority(db,session,state).await?;
                let binding=state.aurinko.as_ref().ok_or_else(invalid)?;
                let owner=state.target_user_id.as_deref().ok_or_else(invalid)?;
                if state.expires_at<=Utc::now() {return Err(invalid());}
                let result=db.collection::<UserApiKey>(KEYS).update_one(doc! {"_id":&binding.key_id,"user_id":owner,"connection_id":&state.connection_id,"oauth_attempt_nonce":&state.id,"credential_epoch":binding.credential_epoch,"status":{"$ne":"revoked"},"credential_source":"platform"},
                    vec![doc! {"$set":{"access_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted.clone()},"credential_encrypted":null,"refresh_token_encrypted":null,"expires_at":null,"status":"active","error_message":null,
                        "token_scopes":{"$literal":scopes.as_str()},"aurinko_account":{"$literal":bson::to_bson(&account).map_err(|_|invalid())?},"last_authorized_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),"credential_epoch":{"$add":[{"$ifNull":["$credential_epoch",1]},1]}}},doc! {"$unset":"oauth_attempt_nonce"}])
                    .session(&mut *session).await?;
                if result.matched_count!=1 {return Err(changed());}
                // Fence snapshots already held by inline receive/reply work.
                db.collection::<Document>("channel_bots").update_many(doc! {"platform":"aurinko","credential_source":"connection","connection_id":&binding.key_id,"user_id":owner,"is_active":true},
                    vec![doc! {"$set":{"updated_at":{"$max":[bson::DateTime::now(),{"$add":["$updated_at",1]}]}}}]).session(&mut *session).await?;
                if let Some(link_id)=&state.connect_link_id {
                    db.collection::<Document>("connect_links").update_one(doc! {"_id":link_id,"status":"pending"},
                        doc! {"$set":{"status":"completed","completed_at":bson::DateTime::now()},"$unset":{"last_error":"","last_error_at":""}}).session(&mut *session).await?;
                }
                db.collection::<OAuthState>(STATES).delete_one(doc! {"_id":&state.id,"consumed":true}).session(&mut *session).await?;
                Ok(())
            }.await;
            transactions::transaction_result(operation)
        })).await.map_err(transactions::map_transaction_error)
    }.await;
    if result.is_err() {
        db.collection::<UserApiKey>(KEYS).update_one(doc! {"_id":&binding.key_id,"oauth_attempt_nonce":&state.id,"status":"pending_auth"},doc! {"$set":{"status":"failed","error_message":"Mailbox authorization failed or cancelled; try again"},"$unset":{"oauth_attempt_nonce":""}}).await?;
    }
    result
}

pub async fn require_live_connection(
    db: &Database,
    owner: &str,
    key_id: &str,
    active: bool,
) -> AppResult<UserApiKey> {
    let mut filter = doc! {"_id":key_id,"user_id":owner,"credential_type":"oauth2","credential_source":"platform","status":{"$ne":"revoked"}};
    if active {
        filter.insert("status", "active");
    }
    let key = db
        .collection::<UserApiKey>(KEYS)
        .find_one(filter)
        .await?
        .ok_or_else(changed)?;
    let provider = provider(db).await?;
    if key.provider_config_id.as_deref() != Some(&provider.id)
        || (active && key.aurinko_account.is_none())
    {
        return Err(changed());
    }
    let services: Vec<UserService> = db
        .collection::<UserService>(SERVICES)
        .find(doc! {"api_key_id":key_id})
        .limit(2)
        .await?
        .try_collect()
        .await?;
    if services.len() != 1
        || services[0].user_id != owner
        || !services[0].is_active
        || db
            .collection::<Document>("users")
            .find_one(doc! {"_id":owner,"is_active":true})
            .await?
            .is_none()
    {
        return Err(changed());
    }
    Ok(key)
}

#[derive(Default)]
pub(crate) struct AurinkoClient {
    #[cfg(test)]
    pub origin: Option<String>,
}
impl AurinkoClient {
    fn url(&self, segments: &[&str]) -> AppResult<url::Url> {
        #[cfg(test)]
        let origin = self.origin.as_deref().unwrap_or(ORIGIN);
        #[cfg(not(test))]
        let origin = ORIGIN;
        let mut url = url::Url::parse(origin).map_err(|_| upstream())?;
        {
            let mut path = url.path_segments_mut().map_err(|_| upstream())?;
            path.clear().push("v1");
            for segment in segments {
                if matches!(*segment, "" | "." | "..") {
                    return Err(invalid());
                }
                path.push(segment);
            }
        }
        Ok(url)
    }
    fn http() -> AppResult<reqwest::Client> {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(3))
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|_| upstream())
    }
    async fn json<T: serde::de::DeserializeOwned>(
        request: reqwest::RequestBuilder,
    ) -> AppResult<T> {
        let mut response = request.send().await.map_err(|_| upstream())?;
        if !response.status().is_success() || response.content_length().is_some_and(|n| n > 65536) {
            return Err(upstream());
        }
        let mut body = Zeroizing::new(Vec::new());
        while let Some(chunk) = response.chunk().await.map_err(|_| upstream())? {
            if body.len() + chunk.len() > 65536 {
                return Err(upstream());
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body).map_err(|_| upstream())
    }
    async fn exchange(
        &self,
        code: &str,
        client_id: &str,
        secret: &str,
    ) -> AppResult<(Zeroizing<String>, String)> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct TokenResponse {
            access_token: Zeroizing<String>,
            account_id: i64,
        }
        let response: TokenResponse = Self::json(
            Self::http()?
                .post(self.url(&["auth", "token", code])?)
                .basic_auth(client_id, Some(secret)),
        )
        .await?;
        if response.account_id <= 0
            || response.access_token.is_empty()
            || response.access_token.len() > 16384
            || response.access_token.chars().any(char::is_whitespace)
        {
            return Err(upstream());
        }
        Ok((response.access_token, response.account_id.to_string()))
    }
    pub(crate) async fn account(
        &self,
        token: &str,
        expected: &str,
        service_type: &str,
        required: &[&str],
    ) -> AppResult<(AurinkoAccount, String)> {
        let mut request = Self::http()?
            .get(self.url(&["account"])?)
            .bearer_auth(token);
        if matches!(service_type, "Google" | "Office365" | "IMAP") {
            request = request.query(&[("pingProvider", "true")]);
        }
        let value = Self::json(request).await?;
        verify_account(&value, expected, service_type, required)
    }
}
fn account_id(value: &Value) -> Option<String> {
    value
        .as_i64()
        .filter(|n| *n > 0)
        .map(|n| n.to_string())
        .or_else(|| {
            value.as_str().and_then(|s| {
                s.parse::<i64>()
                    .ok()
                    .filter(|n| *n > 0 && n.to_string() == s)
                    .map(|n| n.to_string())
            })
        })
}
fn verify_account(
    value: &Value,
    expected: &str,
    service_type: &str,
    required: &[&str],
) -> AppResult<(AurinkoAccount, String)> {
    let id = account_id(&value["id"]).ok_or_else(upstream)?;
    if id != expected
        || value["serviceType"] != service_type
        || value["tokenStatus"] != "active"
        || value["active"] == false
        || value["daemon"] == true
    {
        return Err(AppError::ValidationError(
            "Aurinko returned a different or inactive mailbox account".into(),
        ));
    }
    let scopes = value["authScopes"]
        .as_array()
        .ok_or_else(upstream)?
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    let has = |s: &str| {
        scopes.contains(&"Mail.All")
            || scopes.contains(&s)
            || (s == "Mail.Read" && scopes.contains(&"Mail.ReadWrite"))
            || (s == "Mail.Send" && scopes.contains(&"Mail.Drafts"))
    };
    if required.iter().any(|s| !has(s)) {
        return Err(AppError::ValidationError(
            "Mailbox did not grant the required mail permissions".into(),
        ));
    }
    let address = ["mailboxAddress", "email"]
        .into_iter()
        .find_map(|field| {
            value[field].as_str().filter(|s| {
                s.len() <= 320
                    && s.contains('@')
                    && !s.chars().any(|c| c.is_control() || c.is_whitespace())
            })
        })
        .ok_or_else(upstream)?;
    // Store the granted scopes, expanding documented Mail.All/ReadWrite implications
    // so the existing generic connection scope check has the same meaning.
    let mut scopes = scopes.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    for scope in required {
        if !scopes.iter().any(|s| s == scope) {
            scopes.push(scope.to_string());
        }
    }
    Ok((
        AurinkoAccount {
            application_id_hash: String::new(),
            account_id: id,
            service_type: service_type.into(),
            mailbox_address: address.into(),
        },
        scopes.join(" "),
    ))
}

pub async fn cancel(db: &Database, actor: &str, session_id: &str, attempt: &str) -> AppResult<()> {
    let state_id = format!("1cc_{attempt}");
    let Some(state) = db
        .collection::<OAuthState>(STATES)
        .find_one(doc! {"_id":state_id,"user_id":actor,"aurinko.session_id":session_id})
        .await?
    else {
        return Ok(());
    };
    let binding = state.aurinko.as_ref().ok_or_else(invalid)?;
    // Cancellation must invalidate the nonce even while the exchange owns the
    // effect claim. Its transactional CAS then refuses the late response.
    let db = db.clone();
    let state = state.clone();
    let binding = binding.clone();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run(
            (db.clone(), state, binding),
            |session, (db, state, binding)| {
                Box::pin(async move {
                    let operation: AppResult<()> = async {
                        db.collection::<OAuthState>(STATES)
                            .delete_one(doc! {"_id":&state.id})
                            .session(&mut *session)
                            .await?;
                        db.collection::<UserApiKey>(KEYS)
                            .update_one(
                                doc! {"_id":&binding.key_id,"oauth_attempt_nonce":&state.id},
                                vec![
                                    doc! {"$set":{"updated_at":bson::DateTime::now()}},
                                    doc! {"$unset":"oauth_attempt_nonce"},
                                ],
                            )
                            .session(&mut *session)
                            .await?;
                        Ok(())
                    }
                    .await;
                    transactions::transaction_result(operation)
                })
            },
        )
        .await
        .map_err(transactions::map_transaction_error)
}

pub async fn is_managed_key(db: &Database, key: &UserApiKey) -> AppResult<bool> {
    if key.credential_type != "oauth2" || key.credential_source.as_deref() != Some("platform") {
        return Ok(false);
    }
    let Some(id) = &key.provider_config_id else {
        return Ok(false);
    };
    Ok(db
        .collection::<Document>(PROVIDERS)
        .find_one(doc! {"_id":id,"slug":"aurinko"})
        .await?
        .is_some())
}

async fn app_credentials(
    keys: &EncryptionKeys,
    provider: &ProviderConfig,
) -> AppResult<(Zeroizing<String>, Zeroizing<String>)> {
    let decrypt = async |bytes: Option<&[u8]>| {
        let bytes = Zeroizing::new(keys.decrypt(bytes.ok_or_else(invalid)?).await?);
        Ok::<_, AppError>(Zeroizing::new(
            std::str::from_utf8(&bytes)
                .map_err(|_| invalid())?
                .to_string(),
        ))
    };
    Ok((
        decrypt(provider.client_id_encrypted.as_deref()).await?,
        decrypt(provider.client_secret_encrypted.as_deref()).await?,
    ))
}

#[cfg(test)]
#[path = "aurinko_oauth_service_tests.rs"]
mod tests;
