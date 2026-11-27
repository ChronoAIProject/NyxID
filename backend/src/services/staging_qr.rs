//! A production user may authorize exactly one staging device-login decision.
//! The only credential crossing the boundary is an operation-specific assertion.
use bson::{Document, doc};
use chrono::Utc;
use jsonwebtoken::{Algorithm, DecodingKey, Header, Validation};
use mongodb::{ClientSession, Database};
use serde::{Deserialize, Serialize};
use std::{net::IpAddr, time::Duration};
use uuid::Uuid;

use crate::{
    config::AppConfig,
    crypto::jwt::JwtKeys,
    errors::{AppError, AppResult},
    models::{
        auth_device_code::AuthDeviceClientIpAttribution,
        user::{COLLECTION_NAME as USERS, User},
    },
    services::{auth_device_service as device, feature_flag_service as flags, session_authority},
};

const ISSUER: &str = "https://api.nyxid.ai";
const RECEIVER: &str = "https://staging-api.nyxid.ai/api/v1/auth/device/staging/receive";
const PROOF_TYPE: &str = "nyxid-staging-device+jwt";
const LIFETIME: i64 = 20;

#[derive(Clone)]
pub enum Peer {
    Disabled,
    Sender {
        client: reqwest::Client,
        endpoint: String,
    },
    Receiver(DecodingKey),
}

impl Peer {
    pub fn from_environment(config: &AppConfig) -> Result<Self, String> {
        let base = config.base_url.trim_end_matches('/');
        if config.is_production() && matches!(base, ISSUER | "https://nyx-api.chrono-ai.fun") {
            let client = reqwest::Client::builder()
                .https_only(true)
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(8))
                .build()
                .map_err(|_| "Cannot initialize staging QR client")?;
            return Ok(Self::Sender {
                client,
                endpoint: RECEIVER.into(),
            });
        }
        let Some(path) = std::env::var_os("STAGING_QR_PRODUCTION_PUBLIC_KEY_PATH") else {
            return Ok(Self::Disabled);
        };
        if base != "https://staging-api.nyxid.ai" {
            return Err("Staging QR verification requires the staging API origin".into());
        }
        let pem = std::fs::read(path).map_err(|_| "Cannot read production QR verification key")?;
        Ok(Self::Receiver(
            DecodingKey::from_rsa_pem(&pem)
                .map_err(|_| "Invalid production QR verification key")?,
        ))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Preview,
    Approve,
    Deny,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub request_id: Uuid,
    pub production_user: Uuid,
    pub staging_user: Uuid,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub action: Action,
    pub user_code: String,
    pub binding: Option<Binding>,
}

#[derive(Serialize, Deserialize)]
pub struct Proof {
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
    production_user: Uuid,
    email: String,
    mfa_verified: bool,
    viewer_ip: IpAddr,
    viewer_attribution: AuthDeviceClientIpAttribution,
    request: Request,
}

pub fn forbidden() -> AppError {
    AppError::Forbidden("Staging QR sign-in is unavailable for this account".into())
}

pub fn verify(key: &DecodingKey, token: &str) -> AppResult<Proof> {
    let mut policy = Validation::new(Algorithm::RS256);
    policy.leeway = 0;
    policy.set_issuer(&[ISSUER]);
    policy.set_audience(&[RECEIVER]);
    if token.len() > 8192 {
        return Err(forbidden());
    }
    let parsed = jsonwebtoken::decode::<Proof>(token, key, &policy).map_err(|_| forbidden())?;
    let proof = parsed.claims;
    let now = Utc::now().timestamp();
    if parsed.header.typ.as_deref() != Some(PROOF_TYPE)
        || proof.iat > now + 2
        || proof.iat.checked_add(LIFETIME) != Some(proof.exp)
        || proof.exp <= now
    {
        return Err(forbidden());
    }
    Ok(proof)
}

pub async fn sign(
    db: &Database,
    keys: &JwtKeys,
    subject: Uuid,
    authority: session_authority::DerivedMfaAuthority,
    ip: IpAddr,
    attribution: AuthDeviceClientIpAttribution,
    mut request: Request,
) -> AppResult<zeroize::Zeroizing<String>> {
    if authority.session_id.is_none()
        || !flags::personal_flag_enabled(db, &subject.to_string(), flags::STAGING_QR_LOGIN.key)
            .await?
    {
        return Err(forbidden());
    }
    if request
        .binding
        .as_ref()
        .is_some_and(|b| b.production_user != subject)
        || (request.action != Action::Preview && request.binding.is_none())
    {
        return Err(forbidden());
    }
    request.user_code = device::normalize_user_code(&request.user_code)?;
    let user = db
        .collection::<User>(USERS)
        .find_one(doc! {"_id": subject.to_string(),
        "is_active": true, "email_verified": true})
        .await?
        .ok_or_else(forbidden)?;
    super::auth_service::ensure_person_user(&user)?;
    if request.action == Action::Approve {
        session_authority::require_derived_mfa(session_authority::DerivedMfaAuthority::new(
            user.mfa_enabled,
            authority.verified,
        ))?;
    }
    let now = Utc::now().timestamp();
    let proof = Proof {
        iss: ISSUER.into(),
        aud: RECEIVER.into(),
        iat: now,
        exp: now + LIFETIME,
        production_user: subject,
        email: user.email,
        mfa_verified: authority.verified,
        viewer_ip: ip,
        viewer_attribution: attribution,
        request,
    };
    let mut header = Header::new(Algorithm::RS256);
    header.typ = Some(PROOF_TYPE.into());
    jsonwebtoken::encode(&header, &proof, &keys.encoding)
        .map(zeroize::Zeroizing::new)
        .map_err(|_| AppError::Internal("Could not sign staging QR decision".into()))
}

#[derive(Clone)]
pub struct VerifiedMatch {
    pub request_id: String,
    email: String,
}

impl VerifiedMatch {
    /// Fence the email, account status and MFA policy in the transaction that issues the session.
    pub async fn fence(
        &self,
        db: &Database,
        user_id: &str,
        tx: &mut ClientSession,
        verified: bool,
    ) -> AppResult<()> {
        let mut filter = doc! {"_id": user_id, "email": &self.email, "email_verified": true,
        "is_active": true, "user_type": {"$ne": "org"}};
        if !verified {
            filter.insert("mfa_enabled", false);
        }
        let result = db
            .collection::<Document>(USERS)
            .update_one(
                filter,
                doc! {"$set": {"profile_config.security.use_nonce": Uuid::new_v4().to_string()}},
            )
            .session(tx)
            .await?;
        if result.matched_count != 1 {
            return Err(forbidden());
        }
        Ok(())
    }
}

pub enum Outcome {
    Preview(Box<device::PreviewOutput>, Binding),
    Decision,
}

pub async fn apply(
    db: &Database,
    config: &AppConfig,
    keys: &JwtKeys,
    encryption: &crate::crypto::aes::EncryptionKeys,
    hmac: &[u8],
    proof: Proof,
) -> AppResult<Outcome> {
    let request = proof.request;
    let user = db
        .collection::<User>(USERS)
        .find_one(doc! {"email": &proof.email,
        "email_verified": true, "is_active": true, "user_type": {"$ne": "org"}})
        .await?
        .ok_or_else(forbidden)?;
    let normalized = device::normalize_user_code(&request.user_code)?;
    let (_, row) =
        device::find_by_user_code(db, &device::hmac_hex(hmac, normalized.as_bytes())).await?;
    let binding = Binding {
        production_user: proof.production_user,
        staging_user: Uuid::parse_str(&user.id).map_err(|_| forbidden())?,
        request_id: Uuid::parse_str(&row.id).map_err(|_| forbidden())?,
    };
    if request
        .binding
        .as_ref()
        .is_some_and(|expected| expected != &binding)
        || (request.action != Action::Preview && request.binding.is_none())
    {
        return Err(forbidden());
    }
    let ip = Some(proof.viewer_ip.to_string());
    let agent = Some("NyxID production QR approval".to_owned());
    match request.action {
        Action::Preview => {
            let mut preview = device::preview_row(row, ip.as_deref(), proof.viewer_attribution);
            preview.supports_grant_choice = false;
            Ok(Outcome::Preview(Box::new(preview), binding))
        }
        Action::Approve => {
            let authority =
                session_authority::DerivedMfaAuthority::new(user.mfa_enabled, proof.mfa_verified);
            session_authority::require_derived_mfa(authority)?;
            let verified = VerifiedMatch {
                request_id: row.id,
                email: proof.email,
            };
            device::approve_bound(
                db,
                config,
                keys,
                encryption,
                hmac,
                device::ApproveInput {
                    user_id: user.id,
                    user_code: normalized,
                    approver_ip: ip,
                    approver_user_agent: agent,
                },
                authority,
                Some(verified),
            )
            .await?;
            Ok(Outcome::Decision)
        }
        Action::Deny => {
            device::deny_bound(
                db,
                hmac,
                device::DenyInput {
                    user_id: user.id,
                    user_code: normalized,
                    denier_ip: ip,
                    denier_user_agent: agent,
                },
                Some(&row.id),
            )
            .await?;
            Ok(Outcome::Decision)
        }
    }
}
