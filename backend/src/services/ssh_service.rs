use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::Utc;
use mongodb::bson::doc;
use rand::RngCore;
use ssh_key::{Algorithm, LineEnding, PrivateKey, PublicKey, certificate};
use zeroize::Zeroizing;

use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService, SshServiceConfig,
};
use crate::models::ssh_auth_mode::SshAuthMode;

#[derive(Clone)]
pub struct SshSessionManager {
    slots: crate::services::cluster_slot_service::RenewableSlotManager,
    max_sessions_per_user: u32,
}

#[derive(Debug)]
pub struct ResolvedSshAuthContext {
    pub mode: SshAuthMode,
    pub service_slug: String,
    pub owner_user_id: String,
}

impl SshSessionManager {
    pub fn new(
        slots: crate::services::cluster_slot_service::RenewableSlotManager,
        max_sessions_per_user: usize,
    ) -> Self {
        Self {
            slots,
            max_sessions_per_user: u32::try_from(max_sessions_per_user).unwrap_or(u32::MAX),
        }
    }

    pub async fn try_acquire(&self, user_id: &str) -> AppResult<SshSessionGuard> {
        self.slots
            .acquire("ssh_session", user_id, self.max_sessions_per_user)
            .await?
            .ok_or(AppError::RateLimited)
    }
}

pub type SshSessionGuard = crate::services::cluster_slot_service::RenewableSlotGuard;

pub struct IssuedSshCertificate {
    pub key_id: String,
    pub principal: String,
    pub certificate: String,
    pub ca_public_key: String,
    pub valid_after: chrono::DateTime<Utc>,
    pub valid_before: chrono::DateTime<Utc>,
}

pub struct SshConfigInput<'a> {
    pub host: &'a str,
    pub port: u16,
    pub certificate_auth_enabled: bool,
    pub ssh_auth_mode: Option<SshAuthMode>,
    pub certificate_ttl_minutes: u32,
    pub allowed_principals: &'a [String],
}

pub async fn get_ssh_service(
    db: &mongodb::Database,
    service_id: &str,
) -> AppResult<SshServiceConfig> {
    let service = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "_id": service_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound("SSH service not found".to_string()))?;

    ensure_ssh_service(&service).cloned()
}

pub async fn resolve_ssh_auth_context_for_owner(
    db: &mongodb::Database,
    owner_user_id: &str,
    service: &DownstreamService,
) -> AppResult<ResolvedSshAuthContext> {
    if let Some(user_service) = crate::services::user_service_service::find_by_catalog_service_id(
        db,
        owner_user_id,
        &service.id,
    )
    .await?
    {
        return Ok(ResolvedSshAuthContext {
            mode: user_service.ssh_auth_mode,
            service_slug: user_service.slug,
            owner_user_id: user_service.user_id,
        });
    }

    let ssh = ensure_ssh_service(service)?;

    Ok(ResolvedSshAuthContext {
        mode: ssh.ssh_auth_mode,
        service_slug: service.slug.clone(),
        owner_user_id: owner_user_id.to_string(),
    })
}

pub fn ensure_ssh_service(service: &DownstreamService) -> AppResult<&SshServiceConfig> {
    if service.service_type != "ssh" {
        return Err(AppError::NotFound("SSH service not found".to_string()));
    }

    service
        .ssh_config
        .as_ref()
        .ok_or_else(|| AppError::NotFound("SSH service not found".to_string()))
}

pub async fn build_ssh_config(
    encryption_keys: &EncryptionKeys,
    service_id: &str,
    existing: Option<&SshServiceConfig>,
    input: SshConfigInput<'_>,
) -> AppResult<SshServiceConfig> {
    validate_resolved_ssh_target(input.host, input.port).await?;
    let ssh_auth_mode = input.ssh_auth_mode.unwrap_or_else(|| {
        SshAuthMode::from_certificate_auth_enabled(input.certificate_auth_enabled)
    });
    validate_ssh_auth_mode_settings(
        ssh_auth_mode,
        input.certificate_ttl_minutes,
        input.allowed_principals,
    )?;

    let (ca_private_key_encrypted, ca_public_key) = ca_material_for_upsert(
        encryption_keys,
        service_id,
        existing,
        ssh_auth_mode.certificate_auth_enabled(),
    )
    .await?;

    Ok(SshServiceConfig {
        host: input.host.trim().to_string(),
        port: input.port,
        ssh_auth_mode,
        certificate_auth_enabled: ssh_auth_mode.certificate_auth_enabled(),
        certificate_ttl_minutes: input.certificate_ttl_minutes,
        allowed_principals: sanitize_allowed_principals(input.allowed_principals),
        ca_private_key_encrypted,
        ca_public_key,
    })
}

pub fn target_base_url(host: &str, port: u16) -> String {
    format!("ssh://{}:{port}", host.trim())
}

/// Validate an SSH target hostname and port.
///
/// Unlike HTTP base_url validation, SSH targets are always allowed to use
/// private/internal IPs. SSH services are admin-configured infrastructure
/// (not user-supplied URLs), so SSRF is not a concern. The NyxID server or
/// node agent connects to these hosts on behalf of authenticated users.
pub async fn validate_resolved_ssh_target(host: &str, port: u16) -> AppResult<()> {
    validate_ssh_target_syntax(host, port)?;
    Ok(())
}

/// Validate SSH target syntax only (non-empty host, valid port, blocked
/// hostnames like metadata endpoints).
fn validate_ssh_target_syntax(host: &str, port: u16) -> AppResult<()> {
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
    // Still block cloud metadata endpoints (SSRF to metadata is always dangerous)
    if is_blocked_ssh_hostname(trimmed) {
        return Err(AppError::ValidationError(
            "host must not point to a cloud metadata endpoint".to_string(),
        ));
    }
    Ok(())
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn validate_certificate_settings(
    certificate_auth_enabled: bool,
    certificate_ttl_minutes: u32,
    allowed_principals: &[String],
) -> AppResult<()> {
    validate_ssh_auth_mode_settings(
        SshAuthMode::from_certificate_auth_enabled(certificate_auth_enabled),
        certificate_ttl_minutes,
        allowed_principals,
    )
}

pub fn validate_ssh_auth_mode_settings(
    ssh_auth_mode: SshAuthMode,
    certificate_ttl_minutes: u32,
    allowed_principals: &[String],
) -> AppResult<()> {
    if !(15..=60).contains(&certificate_ttl_minutes) {
        return Err(AppError::ValidationError(
            "certificate_ttl_minutes must be between 15 and 60".to_string(),
        ));
    }

    if ssh_auth_mode == SshAuthMode::ProxyOnly {
        return Ok(());
    }

    if allowed_principals.is_empty() {
        return Err(AppError::ValidationError(
            "allowed_principals is required when SSH auth mode is cert or node_key".to_string(),
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

fn sanitize_allowed_principals(principals: &[String]) -> Vec<String> {
    principals
        .iter()
        .map(|principal| principal.trim().to_string())
        .filter(|principal| !principal.is_empty())
        .collect()
}

fn is_blocked_ssh_hostname(host: &str) -> bool {
    let normalized = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    // Block cloud metadata endpoints only -- private IPs/hostnames are allowed
    normalized == "metadata.google.internal"
}

async fn ca_material_for_upsert(
    encryption_keys: &EncryptionKeys,
    service_id: &str,
    existing: Option<&SshServiceConfig>,
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
    ssh_service: &SshServiceConfig,
    service_id: &str,
    user_id: &str,
    user_email: &str,
    public_key_openssh: &str,
    principal: &str,
) -> AppResult<IssuedSshCertificate> {
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
    let decrypted_ca_private_key =
        Zeroizing::new(encryption_keys.decrypt(ca_private_key_encrypted).await?);
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
    cert_builder
        .serial(rng.next_u64())
        .map_err(|e| AppError::Internal(format!("Failed to set SSH certificate serial: {e}")))?;
    cert_builder
        .key_id(format!("nyxid:{service_id}:{user_id}:{principal}"))
        .map_err(|e| AppError::Internal(format!("Failed to set SSH certificate key id: {e}")))?;
    cert_builder
        .cert_type(certificate::CertType::User)
        .map_err(|e| AppError::Internal(format!("Failed to set SSH certificate type: {e}")))?;
    cert_builder
        .valid_principal(principal)
        .map_err(|e| AppError::Internal(format!("Failed to set SSH certificate principal: {e}")))?;
    cert_builder
        .comment(format!("NyxID SSH certificate for {user_email}"))
        .map_err(|e| AppError::Internal(format!("Failed to set SSH certificate comment: {e}")))?;
    let certificate = cert_builder
        .sign(&ca_private_key)
        .map_err(|e| AppError::Internal(format!("Failed to sign SSH certificate: {e}")))?;
    let certificate_openssh = certificate
        .to_openssh()
        .map_err(|e| AppError::Internal(format!("Failed to encode SSH certificate: {e}")))?;

    Ok(IssuedSshCertificate {
        key_id: format!("nyxid:{service_id}:{user_id}:{principal}"),
        principal: principal.to_string(),
        certificate: certificate_openssh,
        ca_public_key,
        valid_after: chrono::DateTime::<Utc>::from(valid_after_time),
        valid_before: chrono::DateTime::<Utc>::from(valid_before_time),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        SshConfigInput, build_ssh_config, issue_certificate, resolve_ssh_auth_context_for_owner,
        target_base_url, validate_certificate_settings, validate_principal,
        validate_ssh_target_syntax,
    };
    use crate::crypto::aes::EncryptionKeys;
    use crate::crypto::local_key_provider::LocalKeyProvider;
    use crate::models::downstream_service::SshServiceConfig;
    use crate::models::ssh_auth_mode::SshAuthMode;
    use std::sync::Arc;

    #[tokio::test]
    async fn catalog_backed_ssh_bills_personal_and_org_user_service_owners() {
        use crate::models::downstream_service::{
            COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService, test_helpers::dummy_service,
        };
        use crate::models::org_membership::{
            COLLECTION_NAME as ORG_MEMBERSHIPS, OrgMembership, OrgRole,
        };
        use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
        use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
        use crate::services::billing::BillingOwnerResolver;
        use crate::test_utils::{
            connect_test_database, test_membership, test_user, test_user_service,
        };

        let Some(db) = connect_test_database("ssh_catalog_billing_owner").await else {
            eprintln!("Skipping MongoDB-backed test; no test database available");
            return;
        };

        let catalog_author_id = uuid::Uuid::new_v4().to_string();
        let personal_owner_id = uuid::Uuid::new_v4().to_string();
        let org_owner_id = uuid::Uuid::new_v4().to_string();
        let org_member_id = uuid::Uuid::new_v4().to_string();
        let catalog_service_id = uuid::Uuid::new_v4().to_string();
        let personal_service_id = uuid::Uuid::new_v4().to_string();
        let org_service_id = uuid::Uuid::new_v4().to_string();

        db.collection::<User>(USERS)
            .insert_many([
                test_user(&catalog_author_id, UserType::Person),
                test_user(&personal_owner_id, UserType::Person),
                test_user(&org_owner_id, UserType::Org),
                test_user(&org_member_id, UserType::Person),
            ])
            .await
            .expect("insert SSH ownership users");

        let mut catalog_service = dummy_service();
        catalog_service.id = catalog_service_id.clone();
        catalog_service.slug = "catalog-ssh".to_string();
        catalog_service.service_type = "ssh".to_string();
        catalog_service.created_by = catalog_author_id.clone();
        catalog_service.ssh_config = Some(SshServiceConfig {
            host: "10.0.0.5".to_string(),
            port: 22,
            ssh_auth_mode: SshAuthMode::Cert,
            certificate_auth_enabled: true,
            certificate_ttl_minutes: 30,
            allowed_principals: vec!["ubuntu".to_string()],
            ca_private_key_encrypted: None,
            ca_public_key: None,
        });
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&catalog_service)
            .await
            .expect("insert admin-authored SSH catalog service");

        let mut personal_service = test_user_service(
            &personal_service_id,
            &personal_owner_id,
            "personal-ssh",
            &uuid::Uuid::new_v4().to_string(),
            Some(&catalog_service_id),
            None,
        );
        personal_service.service_type = "ssh".to_string();
        personal_service.ssh_auth_mode = SshAuthMode::Cert;
        let mut org_service = test_user_service(
            &org_service_id,
            &org_owner_id,
            "org-ssh",
            &uuid::Uuid::new_v4().to_string(),
            Some(&catalog_service_id),
            None,
        );
        org_service.service_type = "ssh".to_string();
        org_service.ssh_auth_mode = SshAuthMode::NodeKey;
        db.collection::<UserService>(USER_SERVICES)
            .insert_many([personal_service, org_service])
            .await
            .expect("insert catalog-backed SSH user services");
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(
                &org_owner_id,
                &org_member_id,
                OrgRole::Member,
                None,
            ))
            .await
            .expect("insert SSH org membership");

        let personal_context =
            resolve_ssh_auth_context_for_owner(&db, &personal_owner_id, &catalog_service)
                .await
                .expect("resolve personal catalog-backed SSH context");
        let org_context = resolve_ssh_auth_context_for_owner(&db, &org_owner_id, &catalog_service)
            .await
            .expect("resolve org catalog-backed SSH context");

        assert_eq!(personal_context.owner_user_id, personal_owner_id);
        assert_eq!(personal_context.service_slug, "personal-ssh");
        assert_eq!(org_context.owner_user_id, org_owner_id);
        assert_eq!(org_context.service_slug, "org-ssh");
        assert_ne!(personal_context.owner_user_id, catalog_author_id);
        assert_ne!(org_context.owner_user_id, catalog_author_id);

        let billing = BillingOwnerResolver::new(db);
        let personal_payer = billing
            .resolve_for_resource(&personal_owner_id, &personal_context.owner_user_id)
            .await
            .expect("resolve personal SSH payer");
        let org_payer = billing
            .resolve_for_resource(&org_member_id, &org_context.owner_user_id)
            .await
            .expect("resolve org SSH payer");
        assert_eq!(personal_payer.owner_id, personal_owner_id);
        assert_eq!(org_payer.owner_id, org_owner_id);
    }

    #[test]
    fn validates_ssh_target_syntax() {
        assert!(validate_ssh_target_syntax("ssh.internal.example", 22).is_ok());
        assert!(validate_ssh_target_syntax("", 22).is_err());
        assert!(validate_ssh_target_syntax("ssh.internal.example", 0).is_err());
        // Private/internal IPs are allowed for SSH targets
        assert!(validate_ssh_target_syntax("127.0.0.1", 22).is_ok());
        assert!(validate_ssh_target_syntax("100.64.0.10", 22).is_ok());
        assert!(validate_ssh_target_syntax("192.168.1.50", 22).is_ok());
        assert!(validate_ssh_target_syntax("[::1]", 22).is_ok());
        // Cloud metadata endpoints are still blocked
        assert!(validate_ssh_target_syntax("metadata.google.internal", 22).is_err());
    }

    #[test]
    fn validates_certificate_settings() {
        assert!(validate_certificate_settings(false, 30, &[]).is_ok());
        assert!(validate_certificate_settings(true, 30, &[String::from("ubuntu")]).is_ok());
        assert!(validate_certificate_settings(true, 10, &[String::from("ubuntu")]).is_err());
        assert!(validate_certificate_settings(true, 30, &[]).is_err());
    }

    #[test]
    fn validates_principal() {
        assert!(validate_principal("ubuntu").is_ok());
        assert!(validate_principal("deploy.user@example.com").is_ok());
        assert!(validate_principal("bad principal").is_err());
    }

    #[tokio::test]
    async fn builds_ssh_config_and_preserves_existing_ca() {
        let encryption_keys =
            EncryptionKeys::with_provider(Arc::new(LocalKeyProvider::new([7_u8; 32], None)));
        let existing = SshServiceConfig {
            host: "old.example".to_string(),
            port: 22,
            ssh_auth_mode: SshAuthMode::Cert,
            certificate_auth_enabled: true,
            certificate_ttl_minutes: 30,
            allowed_principals: vec!["ubuntu".to_string()],
            ca_private_key_encrypted: Some(vec![1, 2, 3]),
            ca_public_key: Some("ssh-ed25519 AAAAexisting".to_string()),
        };

        let updated = build_ssh_config(
            &encryption_keys,
            "service-1",
            Some(&existing),
            SshConfigInput {
                host: "ssh.internal.example",
                port: 2222,
                certificate_auth_enabled: true,
                ssh_auth_mode: None,
                certificate_ttl_minutes: 45,
                allowed_principals: &[String::from("ubuntu"), String::from(" deploy ")],
            },
        )
        .await
        .expect("config");

        assert_eq!(updated.host, "ssh.internal.example");
        assert_eq!(updated.port, 2222);
        assert_eq!(updated.allowed_principals, vec!["ubuntu", "deploy"]);
        assert_eq!(updated.ca_public_key, existing.ca_public_key);
        assert_eq!(
            updated.ca_private_key_encrypted,
            existing.ca_private_key_encrypted
        );
    }

    #[tokio::test]
    async fn issues_short_lived_certificate() {
        let encryption_keys =
            EncryptionKeys::with_provider(Arc::new(LocalKeyProvider::new([42_u8; 32], None)));
        let ssh_service = build_ssh_config(
            &encryption_keys,
            "service-1",
            None,
            SshConfigInput {
                host: "ssh.internal.example",
                port: 22,
                certificate_auth_enabled: true,
                ssh_auth_mode: None,
                certificate_ttl_minutes: 30,
                allowed_principals: &[String::from("ubuntu")],
            },
        )
        .await
        .expect("ssh config");

        let mut rng = rand::rngs::OsRng;
        let public_key = ssh_key::PrivateKey::random(&mut rng, ssh_key::Algorithm::Ed25519)
            .expect("subject key")
            .public_key()
            .to_openssh()
            .expect("openssh");

        let issued = issue_certificate(
            &encryption_keys,
            &ssh_service,
            "service-1",
            "user-1",
            "operator@example.com",
            &public_key,
            "ubuntu",
        )
        .await
        .expect("certificate");

        assert!(
            issued
                .certificate
                .starts_with("ssh-ed25519-cert-v01@openssh.com")
        );
        assert!(issued.ca_public_key.starts_with("ssh-ed25519 "));
        assert!(issued.valid_before > issued.valid_after);
    }

    #[test]
    fn derives_ssh_base_url() {
        assert_eq!(
            target_base_url("ssh.internal.example", 22),
            "ssh://ssh.internal.example:22"
        );
    }
}
