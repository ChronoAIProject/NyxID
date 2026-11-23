//! Exact, short-lived read authority emitted only after proxy admission.
//! Never a bearer grant, delegated command, or substitute for proxy approvals.

use chrono::Utc;
use jsonwebtoken::{Algorithm, Header, Validation, decode, decode_header, encode};
use mongodb::{Database, bson::doc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService},
        oauth_client::OauthClient,
        user_service::{COLLECTION_NAME as CONNECTIONS, UserService},
    },
    mw::auth::{AuthMethod, AuthUser},
};

mod authority;
mod descriptor;
pub use descriptor::{IntrospectionInput, ReadDescriptor};

pub const HEADER: &str = "x-nyxid-resource-request";
const JWT_TYPE: &str = "nyxid-resource-request+jwt";
const MAX_TOKEN_BYTES: usize = 16 * 1024;

fn refused() -> AppError {
    AppError::Unauthorized("Resource request proof is not active".into())
}

#[derive(Serialize, Deserialize)]
struct Claims {
    #[serde(flatten)]
    identity: super::identity_service::IdentityAssertionClaims,
    nyx_request: ReadDescriptor,
    nyx_authority: authority::AuthorityBinding,
}

/// Constructed only in the proxy path after it has accepted the read. The
/// signer is carried to the final built HTTP request, never exposed to callers.
pub(crate) struct ReadProofIssuer<'a> {
    db: &'a Database,
    jwt_keys: &'a crate::crypto::jwt::JwtKeys,
    config: &'a crate::config::AppConfig,
    subject: String,
    expected_url: reqwest::Url,
    authority: authority::AuthorityBinding,
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn prepare<'a>(
    state: &'a AppState,
    auth: &AuthUser,
    connection_id: Option<&str>,
    target: &super::proxy_service::ProxyTarget,
    approval_service_id: &str,
    method: &str,
    path: &str,
    query: Option<&str>,
    direct_admitted_read: bool,
) -> AppResult<Option<ReadProofIssuer<'a>>> {
    if auth.auth_method != AuthMethod::ApiKey || auth.assistant_chat.is_none() {
        return Ok(None);
    }
    prepare_admitted(
        &state.db,
        &state.jwt_keys,
        &state.config,
        &auth.user_id.to_string(),
        auth.api_key_id.as_deref(),
        auth.api_key_credential_id.as_deref(),
        connection_id,
        target,
        approval_service_id,
        method,
        path,
        query,
        direct_admitted_read,
    )
    .await
}

/// Both HTTP and NyxBot MCP use this after their existing admission and approval
/// gates. IDs come from authentication, never from tool or request arguments.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn prepare_admitted<'a>(
    db: &'a Database,
    jwt_keys: &'a crate::crypto::jwt::JwtKeys,
    config: &'a crate::config::AppConfig,
    subject: &str,
    key_id: Option<&str>,
    credential_id: Option<&str>,
    connection_id: Option<&str>,
    target: &super::proxy_service::ProxyTarget,
    approval_service_id: &str,
    method: &str,
    path: &str,
    query: Option<&str>,
    direct_admitted_read: bool,
) -> AppResult<Option<ReadProofIssuer<'a>>> {
    if !config.resource_request_proof_enabled
        || !direct_admitted_read
        || !matches!(method, "GET" | "HEAD")
    {
        return Ok(None);
    }
    let Some(key_id) = key_id else {
        return Ok(None);
    };
    let Some(connection_id) = connection_id else {
        return Ok(None);
    };
    let connection = db
        .collection::<UserService>(CONNECTIONS)
        .find_one(doc! {"_id": connection_id, "is_active": true})
        .await?
        .ok_or_else(refused)?;
    let Some(catalog_id) = connection.catalog_service_id.as_deref() else {
        return Ok(None);
    };
    let Some(service) = db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id": catalog_id, "is_active": true})
        .await?
    else {
        return Ok(None);
    };
    // Registration is catalog-owned. UserService identity overrides cannot
    // turn an arbitrary downstream destination into a trusted resource server.
    if service
        .introspection_client_ids
        .as_ref()
        .is_none_or(Vec::is_empty)
        || !matches!(service.identity_propagation_mode.as_str(), "jwt" | "both")
    {
        return Ok(None);
    }
    if target.base_url.trim_end_matches('/') != service.base_url.trim_end_matches('/')
        || target.auth_method != "none"
        || !target.credential.is_empty()
    {
        return Err(refused());
    }
    let binding = authority::load(
        db,
        subject,
        key_id,
        credential_id,
        connection_id,
        catalog_id,
        approval_service_id,
        method,
        path,
    )
    .await?;
    // Match the proxy builder exactly, before any delegated/auth transform.
    // V1 permits neither such credentials nor a different final destination.
    let prepared = super::proxy_service::prepare_delegated_request(path, query, &[])?;
    let mut url = format!(
        "{}/{}",
        target.base_url.trim_end_matches('/'),
        prepared.path
    );
    if let Some(query) = prepared.query {
        url.push('?');
        url.push_str(&query);
    }
    let expected_url = url.parse().map_err(|_| refused())?;
    Ok(Some(ReadProofIssuer {
        db,
        jwt_keys,
        config,
        subject: subject.into(),
        expected_url,
        authority: binding,
    }))
}

impl ReadProofIssuer<'_> {
    /// Final server-owned overwrite, after all catalog/user/default headers and
    /// final egress checks. The caller must execute this request without redirects.
    pub(crate) async fn attach(&self, request: &mut reqwest::Request) -> AppResult<()> {
        let descriptor = ReadDescriptor::from_request(request)?;
        if request.url() != &self.expected_url || descriptor.method != self.authority.method {
            return Err(refused());
        }
        self.authority
            .require_current(self.db, &self.subject)
            .await?;
        let now = Utc::now().timestamp();
        let claims = Claims {
            identity: super::identity_service::IdentityAssertionClaims {
                sub: self.subject.clone(),
                iss: self.config.jwt_issuer.clone(),
                aud: self.authority.catalog_id.clone(),
                exp: now + 60,
                iat: now,
                jti: Uuid::new_v4().to_string(),
                email: None,
                name: None,
                nyx_service_id: self.authority.catalog_id.clone(),
                roles: None,
                groups: None,
                permissions: None,
            },
            nyx_request: descriptor,
            nyx_authority: self.authority.clone(),
        };
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(self.jwt_keys.kid.clone());
        header.typ = Some(JWT_TYPE.into());
        let token = encode(&header, &claims, &self.jwt_keys.encoding)
            .map_err(|_| AppError::Internal("Resource request signing failed".into()))?;
        let mut value: reqwest::header::HeaderValue = token.parse().map_err(|_| refused())?;
        value.set_sensitive(true);
        request.headers_mut().insert(HEADER, value);
        Ok(())
    }
}

#[derive(Serialize)]
pub struct Evidence {
    active: bool,
    token_type: &'static str,
    aud: String,
    sub: String,
    iss: String,
    scope: &'static str,
    exp: i64,
    iat: i64,
    jti: String,
    permissions: Vec<String>,
    resource_request: ReadDescriptor,
    independent_commands: bool,
}

pub async fn introspect(
    state: &AppState,
    client: &OauthClient,
    service_id: &str,
    token: &str,
    descriptor: ReadDescriptor,
) -> AppResult<Evidence> {
    if !state.config.resource_request_proof_enabled
        || token.len() > MAX_TOKEN_BYTES
        || client.client_type != "confidential"
        || !client.is_active
    {
        return Err(refused());
    }
    descriptor.validate()?;
    let header = decode_header(token).map_err(|_| refused())?;
    if header.alg != Algorithm::RS256 || header.typ.as_deref() != Some(JWT_TYPE) {
        return Err(refused());
    }
    // Use the same explicit verifier overlap as other NyxID JWT consumers;
    // unknown/missing key IDs never fall back to the active signing key.
    let decoding = state.jwt_keys.decoding_key(token).map_err(|_| refused())?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&[service_id]);
    validation.set_issuer(&state.config.accepted_jwt_issuers());
    validation.leeway = 0;
    let claims = decode::<Claims>(token, decoding, &validation)
        .map_err(|_| refused())?
        .claims;
    let now = Utc::now().timestamp();
    if claims.identity.nyx_service_id != service_id
        || claims.nyx_authority.catalog_id != service_id
        || claims.nyx_request != descriptor
        || claims.identity.exp <= now
        || claims.identity.exp - claims.identity.iat != 60
        || claims.identity.iat > now + 5
        || Uuid::parse_str(&claims.identity.jti).is_err()
    {
        return Err(refused());
    }
    let service = authority::catalog(&state.db, service_id).await?;
    if !service
        .introspection_client_ids
        .as_ref()
        .is_some_and(|ids| ids.contains(&client.id))
    {
        return Err(refused());
    }
    claims
        .nyx_authority
        .require_current(&state.db, &claims.identity.sub)
        .await?;
    let rbac = super::rbac_helpers::resolve_user_rbac(&state.db, &claims.identity.sub).await?;
    Ok(Evidence {
        active: true,
        token_type: "resource_request",
        aud: service_id.into(),
        sub: claims.identity.sub,
        iss: claims.identity.iss,
        scope: "proxy",
        exp: claims.identity.exp,
        iat: claims.identity.iat,
        jti: claims.identity.jti,
        permissions: rbac.permissions,
        resource_request: descriptor,
        independent_commands: false,
    })
}

#[cfg(test)]
mod tests;
