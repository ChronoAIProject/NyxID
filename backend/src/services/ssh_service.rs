use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::Utc;
use dashmap::DashMap;
use mongodb::bson::doc;
use ssh_key::{Algorithm, LineEnding, PrivateKey, PublicKey, certificate};

use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::ssh_service::{COLLECTION_NAME as SSH_SERVICES, SshService};

#[derive(Debug)]
pub struct SshSessionManager {
    concurrent_by_user: Arc<DashMap<String, usize>>,
    max_sessions_per_user: usize,
}

impl SshSessionManager {
    pub fn new(max_sessions_per_user: usize) -> Self {
        Self {
            concurrent_by_user: Arc::new(DashMap::new()),
            max_sessions_per_user,
        }
    }

    pub fn try_acquire(&self, user_id: &str) -> AppResult<SshSessionGuard> {
        let mut entry = self
            .concurrent_by_user
            .entry(user_id.to_string())
            .or_insert(0);
        if *entry >= self.max_sessions_per_user {
            return Err(AppError::RateLimited);
        }

        *entry += 1;
        drop(entry);

        Ok(SshSessionGuard {
            manager: self.concurrent_by_user.clone(),
            user_id: user_id.to_string(),
        })
    }

    pub fn active_sessions_for_user(&self, user_id: &str) -> usize {
        self.concurrent_by_user
            .get(user_id)
            .map(|entry| *entry)
            .unwrap_or(0)
    }
}

pub struct SshSessionGuard {
    manager: Arc<DashMap<String, usize>>,
    user_id: String,
}

impl Drop for SshSessionGuard {
    fn drop(&mut self) {
        if let Some(mut entry) = self.manager.get_mut(&self.user_id) {
            if *entry > 1 {
                *entry -= 1;
            } else {
                drop(entry);
                self.manager.remove(&self.user_id);
            }
        }
    }
}

pub struct IssuedSshCertificate {
    pub key_id: String,
    pub principal: String,
    pub certificate: String,
    pub ca_public_key: String,
    pub valid_after: chrono::DateTime<Utc>,
    pub valid_before: chrono::DateTime<Utc>,
}

pub struct UpsertSshServiceInput<'a> {
    pub host: &'a str,
    pub port: u16,
    pub certificate_auth_enabled: bool,
    pub certificate_ttl_minutes: u32,
    pub allowed_principals: &'a [String],
}

pub async fn get_ssh_service(db: &mongodb::Database, service_id: &str) -> AppResult<SshService> {
    db.collection::<SshService>(SSH_SERVICES)
        .find_one(doc! { "_id": service_id, "enabled": true })
        .await?
        .ok_or_else(|| AppError::NotFound("SSH service not found".to_string()))
}

pub async fn get_ssh_service_optional(
    db: &mongodb::Database,
    service_id: &str,
) -> AppResult<Option<SshService>> {
    db.collection::<SshService>(SSH_SERVICES)
        .find_one(doc! { "_id": service_id })
        .await
        .map_err(Into::into)
}

pub async fn upsert_ssh_service(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    service_id: &str,
    created_by: &str,
    input: UpsertSshServiceInput<'_>,
) -> AppResult<SshService> {
    validate_ssh_target(input.host, input.port)?;
    validate_certificate_settings(
        input.certificate_auth_enabled,
        input.certificate_ttl_minutes,
        input.allowed_principals,
    )?;

    let now = Utc::now();
    let existing = get_ssh_service_optional(db, service_id).await?;
    let (ca_private_key_encrypted, ca_public_key) = ca_material_for_upsert(
        encryption_keys,
        service_id,
        existing.as_ref(),
        input.certificate_auth_enabled,
    )
    .await?;

    let service = match existing {
        Some(existing) => SshService {
            id: existing.id,
            host: input.host.trim().to_string(),
            port: input.port,
            enabled: true,
            certificate_auth_enabled: input.certificate_auth_enabled,
            certificate_ttl_minutes: input.certificate_ttl_minutes,
            allowed_principals: input.allowed_principals.to_vec(),
            ca_private_key_encrypted,
            ca_public_key,
            created_by: existing.created_by,
            created_at: existing.created_at,
            updated_at: now,
        },
        None => SshService {
            id: service_id.to_string(),
            host: input.host.trim().to_string(),
            port: input.port,
            enabled: true,
            certificate_auth_enabled: input.certificate_auth_enabled,
            certificate_ttl_minutes: input.certificate_ttl_minutes,
            allowed_principals: input.allowed_principals.to_vec(),
            ca_private_key_encrypted,
            ca_public_key,
            created_by: created_by.to_string(),
            created_at: now,
            updated_at: now,
        },
    };

    db.collection::<SshService>(SSH_SERVICES)
        .replace_one(doc! { "_id": service_id }, &service)
        .upsert(true)
        .await?;

    Ok(service)
}

pub async fn disable_ssh_service(db: &mongodb::Database, service_id: &str) -> AppResult<()> {
    let result = db
        .collection::<SshService>(SSH_SERVICES)
        .update_one(
            doc! { "_id": service_id },
            doc! {
                "$set": {
                    "enabled": false,
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                }
            },
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound("SSH service not found".to_string()));
    }

    Ok(())
}

pub fn validate_ssh_target(host: &str, port: u16) -> AppResult<()> {
    let trimmed = host.trim();
    if trimmed.is_empty() || trimmed.len() > 255 {
        return Err(AppError::ValidationError(
            "host must be between 1 and 255 characters".to_string(),
        ));
    }

    if port == 0 {
        return Err(AppError::ValidationError(
            "port must be greater than 0".to_string(),
        ));
    }

    Ok(())
}

pub fn validate_certificate_settings(
    certificate_auth_enabled: bool,
    certificate_ttl_minutes: u32,
    allowed_principals: &[String],
) -> AppResult<()> {
    if !(15..=60).contains(&certificate_ttl_minutes) {
        return Err(AppError::ValidationError(
            "certificate_ttl_minutes must be between 15 and 60".to_string(),
        ));
    }

    if !certificate_auth_enabled {
        return Ok(());
    }

    if allowed_principals.is_empty() {
        return Err(AppError::ValidationError(
            "allowed_principals is required when certificate_auth_enabled is true".to_string(),
        ));
    }

    for principal in allowed_principals {
        validate_principal(principal)?;
    }

    Ok(())
}

pub fn validate_principal(principal: &str) -> AppResult<()> {
    let trimmed = principal.trim();
    if trimmed.is_empty() || trimmed.len() > 128 {
        return Err(AppError::ValidationError(
            "principal must be between 1 and 128 characters".to_string(),
        ));
    }

    if !trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '@'))
    {
        return Err(AppError::ValidationError(
            "principal contains unsupported characters".to_string(),
        ));
    }

    Ok(())
}

async fn ca_material_for_upsert(
    encryption_keys: &EncryptionKeys,
    service_id: &str,
    existing: Option<&SshService>,
    certificate_auth_enabled: bool,
) -> AppResult<(Option<Vec<u8>>, Option<String>)> {
    if let Some(existing) = existing
        && (existing.ca_private_key_encrypted.is_some() || existing.ca_public_key.is_some())
    {
        return Ok((
            existing.ca_private_key_encrypted.clone(),
            existing.ca_public_key.clone(),
        ));
    }

    if !certificate_auth_enabled {
        return Ok((None, None));
    }

    generate_service_ca(encryption_keys, service_id).await
}

async fn generate_service_ca(
    encryption_keys: &EncryptionKeys,
    service_id: &str,
) -> AppResult<(Option<Vec<u8>>, Option<String>)> {
    let mut rng = rand::rngs::OsRng;
    let mut ca_key = PrivateKey::random(&mut rng, Algorithm::Ed25519)
        .map_err(|e| AppError::Internal(format!("Failed to generate SSH CA key: {e}")))?;
    ca_key.set_comment(format!("nyxid-ssh-ca:{service_id}"));

    let ca_private_pem = ca_key
        .to_openssh(LineEnding::LF)
        .map_err(|e| AppError::Internal(format!("Failed to encode SSH CA key: {e}")))?;
    let ca_public_key = ca_key
        .public_key()
        .to_openssh()
        .map_err(|e| AppError::Internal(format!("Failed to encode SSH CA public key: {e}")))?;
    let ca_private_key_encrypted = encryption_keys.encrypt(ca_private_pem.as_bytes()).await?;

    Ok((Some(ca_private_key_encrypted), Some(ca_public_key)))
}

pub async fn issue_certificate(
    encryption_keys: &EncryptionKeys,
    ssh_service: &SshService,
    service_id: &str,
    user_id: &str,
    user_email: &str,
    public_key_openssh: &str,
    principal: &str,
) -> AppResult<IssuedSshCertificate> {
    if !ssh_service.enabled {
        return Err(AppError::BadRequest("SSH service is disabled".to_string()));
    }

    if !ssh_service.certificate_auth_enabled {
        return Err(AppError::BadRequest(
            "SSH certificate auth is not enabled for this service".to_string(),
        ));
    }

    validate_principal(principal)?;
    if !ssh_service
        .allowed_principals
        .iter()
        .any(|allowed| allowed == principal)
    {
        return Err(AppError::Forbidden(
            "Requested SSH principal is not allowed for this service".to_string(),
        ));
    }

    let subject_public_key = PublicKey::from_openssh(public_key_openssh.trim())
        .map_err(|e| AppError::ValidationError(format!("Invalid OpenSSH public key: {e}")))?;
    let ca_public_key = ssh_service.ca_public_key.clone().ok_or_else(|| {
        AppError::Internal("SSH certificate CA public key is not configured".to_string())
    })?;
    let ca_private_key_encrypted =
        ssh_service
            .ca_private_key_encrypted
            .as_deref()
            .ok_or_else(|| {
                AppError::Internal("SSH certificate CA private key is not configured".to_string())
            })?;
    let decrypted_ca_private_key = encryption_keys.decrypt(ca_private_key_encrypted).await?;
    let ca_private_key = PrivateKey::from_openssh(&decrypted_ca_private_key)
        .map_err(|e| AppError::Internal(format!("Stored SSH CA private key is invalid: {e}")))?;

    let valid_after_time = SystemTime::now();
    let valid_before_time =
        valid_after_time + Duration::from_secs(ssh_service.certificate_ttl_minutes as u64 * 60);
    let valid_after_secs = valid_after_time
        .duration_since(UNIX_EPOCH)
        .map_err(|e| AppError::Internal(format!("System clock error: {e}")))?
        .as_secs();
    let valid_before_secs = valid_before_time
        .duration_since(UNIX_EPOCH)
        .map_err(|e| AppError::Internal(format!("System clock error: {e}")))?
        .as_secs();

    let mut rng = rand::rngs::OsRng;
    let mut cert_builder = certificate::Builder::new_with_random_nonce(
        &mut rng,
        subject_public_key.key_data().clone(),
        valid_after_secs,
        valid_before_secs,
    )
    .map_err(|e| AppError::Internal(format!("Failed to initialize SSH certificate: {e}")))?;

    let key_id = format!("{service_id}:{user_id}:{principal}:{valid_after_secs}");
    cert_builder
        .serial(rand::random::<u64>())
        .map_err(|e| AppError::Internal(format!("Failed to set SSH cert serial: {e}")))?;
    cert_builder
        .cert_type(certificate::CertType::User)
        .map_err(|e| AppError::Internal(format!("Failed to set SSH cert type: {e}")))?;
    cert_builder
        .key_id(key_id.clone())
        .map_err(|e| AppError::Internal(format!("Failed to set SSH cert key id: {e}")))?;
    cert_builder
        .valid_principal(principal)
        .map_err(|e| AppError::Internal(format!("Failed to set SSH cert principal: {e}")))?;
    cert_builder
        .comment(user_email)
        .map_err(|e| AppError::Internal(format!("Failed to set SSH cert comment: {e}")))?;

    for extension in [
        "permit-agent-forwarding",
        "permit-port-forwarding",
        "permit-pty",
        "permit-user-rc",
    ] {
        cert_builder
            .extension(extension, "")
            .map_err(|e| AppError::Internal(format!("Failed to set SSH cert extension: {e}")))?;
    }

    let certificate = cert_builder
        .sign(&ca_private_key)
        .and_then(|certificate| certificate.to_openssh())
        .map_err(|e| AppError::Internal(format!("Failed to sign SSH certificate: {e}")))?;

    Ok(IssuedSshCertificate {
        key_id,
        principal: principal.to_string(),
        certificate,
        ca_public_key,
        valid_after: chrono::DateTime::<Utc>::from(valid_after_time),
        valid_before: chrono::DateTime::<Utc>::from(valid_before_time),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        SshSessionManager, generate_service_ca, issue_certificate, validate_certificate_settings,
        validate_ssh_target,
    };
    use crate::crypto::aes::EncryptionKeys;
    use crate::crypto::local_key_provider::LocalKeyProvider;
    use crate::models::ssh_service::SshService;
    use chrono::Utc;
    use ssh_key::{Algorithm, PrivateKey};
    use std::sync::Arc;

    #[test]
    fn validates_ssh_target() {
        assert!(validate_ssh_target("ssh.internal", 22).is_ok());
        assert!(validate_ssh_target("", 22).is_err());
    }

    #[test]
    fn enforces_concurrent_session_limit() {
        let manager = SshSessionManager::new(1);
        let guard = manager.try_acquire("user-1").expect("first acquire");
        assert_eq!(manager.active_sessions_for_user("user-1"), 1);
        assert!(manager.try_acquire("user-1").is_err());
        drop(guard);
    }

    #[test]
    fn validates_certificate_settings() {
        assert!(validate_certificate_settings(true, 30, &[String::from("ubuntu")]).is_ok());
        assert!(validate_certificate_settings(true, 10, &[String::from("ubuntu")]).is_err());
        assert!(validate_certificate_settings(true, 30, &[]).is_err());
    }

    #[tokio::test]
    async fn issues_certificate_for_allowed_principal() {
        let encryption_keys =
            EncryptionKeys::with_provider(Arc::new(LocalKeyProvider::new([7_u8; 32], None)));
        let (ca_private_key_encrypted, ca_public_key) =
            generate_service_ca(&encryption_keys, "svc-1")
                .await
                .expect("generate ca");

        let ssh_service = SshService {
            id: "svc-1".to_string(),
            host: "ssh.internal.example".to_string(),
            port: 22,
            enabled: true,
            certificate_auth_enabled: true,
            certificate_ttl_minutes: 30,
            allowed_principals: vec!["ubuntu".to_string()],
            ca_private_key_encrypted,
            ca_public_key,
            created_by: "admin".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let mut rng = rand::rngs::OsRng;
        let mut subject_key =
            PrivateKey::random(&mut rng, Algorithm::Ed25519).expect("subject key");
        subject_key.set_comment("user@example.com");
        let subject_public_key = subject_key
            .public_key()
            .to_openssh()
            .expect("encode public key");

        let issued = issue_certificate(
            &encryption_keys,
            &ssh_service,
            "svc-1",
            "user-1",
            "user@example.com",
            &subject_public_key,
            "ubuntu",
        )
        .await
        .expect("issue certificate");

        assert!(issued.certificate.contains("-cert-v01@openssh.com"));
        assert_eq!(issued.principal, "ubuntu");
        assert!(issued.valid_before > issued.valid_after);
    }
}
