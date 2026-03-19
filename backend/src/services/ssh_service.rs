use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::Utc;
use dashmap::DashMap;
use mongodb::bson::doc;
use rand::RngCore;
use ssh_key::{Algorithm, LineEnding, PrivateKey, PublicKey, certificate};
use zeroize::Zeroizing;

use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService, SshServiceConfig,
};

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
        let _ = self.manager.remove_if_mut(&self.user_id, |_, count| {
            if *count > 1 {
                *count -= 1;
                false
            } else {
                true
            }
        });
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

pub struct SshConfigInput<'a> {
    pub host: &'a str,
    pub port: u16,
    pub certificate_auth_enabled: bool,
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
    validate_certificate_settings(
        input.certificate_auth_enabled,
        input.certificate_ttl_minutes,
        input.allowed_principals,
    )?;

    let (ca_private_key_encrypted, ca_public_key) = ca_material_for_upsert(
        encryption_keys,
        service_id,
        existing,
        input.certificate_auth_enabled,
    )
    .await?;

    Ok(SshServiceConfig {
        host: input.host.trim().to_string(),
        port: input.port,
        certificate_auth_enabled: input.certificate_auth_enabled,
        certificate_ttl_minutes: input.certificate_ttl_minutes,
        allowed_principals: sanitize_allowed_principals(input.allowed_principals),
        ca_private_key_encrypted,
        ca_public_key,
    })
}

pub fn target_base_url(host: &str, port: u16) -> String {
    format!("ssh://{}:{port}", host.trim())
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

    if is_blocked_ssh_hostname(trimmed) {
        return Err(AppError::ValidationError(
            "host must not point to a private or internal address".to_string(),
        ));
    }

    if let Ok(ip) = parse_ssh_target_ip(trimmed)
        && is_private_or_internal_ip(ip)
    {
        return Err(AppError::ValidationError(
            "host must not point to a private or internal IP address".to_string(),
        ));
    }

    Ok(())
}

pub async fn validate_resolved_ssh_target(host: &str, port: u16) -> AppResult<()> {
    validate_resolved_ssh_target_with_lookup(host, port, |lookup_host, lookup_port| {
        let lookup_host = lookup_host.to_string();
        async move {
            let resolved = tokio::net::lookup_host((lookup_host.as_str(), lookup_port)).await?;
            Ok(resolved.map(|addr| addr.ip()).collect())
        }
    })
    .await
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

fn sanitize_allowed_principals(principals: &[String]) -> Vec<String> {
    principals
        .iter()
        .map(|principal| principal.trim().to_string())
        .filter(|principal| !principal.is_empty())
        .collect()
}

fn is_blocked_ssh_hostname(host: &str) -> bool {
    matches!(
        normalize_host(host).as_str(),
        "localhost" | "metadata.google.internal"
    )
}

fn parse_ssh_target_ip(host: &str) -> Result<std::net::IpAddr, std::net::AddrParseError> {
    normalize_host(host).parse()
}

async fn validate_resolved_ssh_target_with_lookup<F, Fut>(
    host: &str,
    port: u16,
    lookup: F,
) -> AppResult<()>
where
    F: Fn(&str, u16) -> Fut,
    Fut: Future<Output = std::io::Result<Vec<std::net::IpAddr>>>,
{
    validate_ssh_target(host, port)?;

    let resolved_ips = match parse_ssh_target_ip(host) {
        Ok(ip) => vec![ip],
        Err(_) => match lookup(host, port).await {
            // Best-effort DNS resolution catches resolvable private targets
            // without blocking split-horizon names only reachable from nodes.
            Ok(addresses) => addresses,
            Err(_) => return Ok(()),
        },
    };

    if resolved_ips.iter().copied().any(is_private_or_internal_ip) {
        return Err(AppError::ValidationError(
            "host must not resolve to a private or internal IP address".to_string(),
        ));
    }

    Ok(())
}

fn normalize_host(host: &str) -> String {
    host.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

fn is_private_or_internal_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ipv4) => {
            ipv4.is_loopback()
                || ipv4.is_private()
                || ipv4.is_link_local()
                || ipv4.is_unspecified()
                || ipv4.is_broadcast()
                || is_rfc6598_cgnat(ipv4)
                || ipv4.octets()[0] == 169 && ipv4.octets()[1] == 254
        }
        std::net::IpAddr::V6(ipv6) => {
            ipv6.is_loopback()
                || ipv6.is_unspecified()
                || (ipv6.segments()[0] & 0xfe00) == 0xfc00
                || (ipv6.segments()[0] & 0xffc0) == 0xfe80
                || ipv6
                    .to_ipv4_mapped()
                    .is_some_and(|mapped| is_private_or_internal_ip(mapped.into()))
        }
    }
}

fn is_rfc6598_cgnat(ipv4: std::net::Ipv4Addr) -> bool {
    ipv4.octets()[0] == 100 && (64..=127).contains(&ipv4.octets()[1])
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
        SshConfigInput, SshSessionManager, build_ssh_config, issue_certificate, target_base_url,
        validate_certificate_settings, validate_principal,
        validate_resolved_ssh_target_with_lookup, validate_ssh_target,
    };
    use crate::crypto::aes::EncryptionKeys;
    use crate::crypto::local_key_provider::LocalKeyProvider;
    use crate::models::downstream_service::SshServiceConfig;
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::Arc;

    #[test]
    fn validates_ssh_target() {
        assert!(validate_ssh_target("ssh.internal.example", 22).is_ok());
        assert!(validate_ssh_target("", 22).is_err());
        assert!(validate_ssh_target("ssh.internal.example", 0).is_err());
        assert!(validate_ssh_target("127.0.0.1", 22).is_err());
        assert!(validate_ssh_target("100.64.0.10", 22).is_err());
        assert!(validate_ssh_target("[::1]", 22).is_err());
        assert!(validate_ssh_target("metadata.google.internal", 22).is_err());
    }

    #[tokio::test]
    async fn rejects_hostnames_that_resolve_to_private_ips() {
        let error = validate_resolved_ssh_target_with_lookup("ssh.example.com", 22, |_, _| async {
            Ok(vec![IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5))])
        })
        .await
        .expect_err("private resolved target");

        assert!(
            error
                .to_string()
                .contains("must not resolve to a private or internal IP address")
        );
    }

    #[tokio::test]
    async fn allows_unresolved_hostnames_for_node_only_targets() {
        validate_resolved_ssh_target_with_lookup("ssh.internal.example", 22, |_, _| async {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "unresolved",
            ))
        })
        .await
        .expect("unresolved host should not fail validation");
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

    #[test]
    fn tracks_concurrent_sessions_per_user() {
        let manager = SshSessionManager::new(2);
        let guard1 = manager.try_acquire("user-1").expect("first");
        let guard2 = manager.try_acquire("user-1").expect("second");
        assert_eq!(manager.active_sessions_for_user("user-1"), 2);
        assert!(manager.try_acquire("user-1").is_err());
        drop(guard1);
        assert_eq!(manager.active_sessions_for_user("user-1"), 1);
        drop(guard2);
        assert_eq!(manager.active_sessions_for_user("user-1"), 0);
    }

    #[tokio::test]
    async fn builds_ssh_config_and_preserves_existing_ca() {
        let encryption_keys =
            EncryptionKeys::with_provider(Arc::new(LocalKeyProvider::new([7_u8; 32], None)));
        let existing = SshServiceConfig {
            host: "old.example".to_string(),
            port: 22,
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
