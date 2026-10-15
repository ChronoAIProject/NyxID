//! Hosted Remote Nyx Machine bootstrap.
//!
//! Kubernetes issues a Pod-bound audience token through a protected projected
//! mount; the optional legacy provider-ticket transport remains supported. NyxID returns credentials sealed to an ephemeral node key and
//! bound to the ticket's Sandbox/Pod/runtime/epoch tuple. The node persists a
//! pending delivery before the request so a lost response can be retried with
//! the same key and delivery id. Plain credentials are written only to the
//! normal encrypted NodeConfig store and never logged or returned.

use anyhow::{Context, Result, ensure};
use clap::Args;
use nyxid_machine::bootstrap as sealed_bootstrap;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};
use zeroize::Zeroizing;

pub(super) use super::managed_config::validate_existing_config;
use super::managed_config::{default_capabilities, machine_config, validate_endpoint};

use super::{
    config::{self, NodeConfig},
    secret_backend::SecretBackend,
};

#[derive(Args, Debug)]
pub struct Start {
    /// Provider-signed ticket file. The file is read once per delivery and is
    /// never copied into the workspace or included in logs.
    #[arg(
        long,
        env = "NYXID_MANAGED_TICKET_FILE",
        required_unless_present = "kubernetes"
    )]
    pub ticket_file: Option<PathBuf>,
    /// Use a projected, Pod-bound Kubernetes service-account identity.
    #[arg(long, env = "NYXID_MANAGED_KUBERNETES", value_parser = clap::builder::BoolishValueParser::new())]
    pub kubernetes: bool,
    /// NyxID managed bootstrap endpoint. This must be HTTPS outside local
    /// development and is never inferred from the ordinary node WSS URL.
    #[arg(long, env = "NYXID_MANAGED_BOOTSTRAP_URL")]
    pub bootstrap_url: String,
    #[arg(long)]
    pub config: Option<String>,
}

/// Prepare a recipient-bound challenge before the trusted platform issues its
/// ticket. This operation creates no Node credential and starts no runtime.
#[derive(Args, Debug)]
pub struct Prepare {
    #[arg(long, env = "NYXID_MANAGED_TICKET_FILE")]
    pub ticket_file: PathBuf,
    #[arg(long)]
    pub config: Option<String>,
}

#[derive(Deserialize)]
struct Ticket {
    pub binding: nyxid_machine::managed::Binding,
    #[serde(default)]
    token: Option<Zeroizing<String>>,
}

#[derive(Clone, Serialize, Deserialize)]
struct PendingDelivery {
    binding: nyxid_machine::managed::Binding,
    private_key: Zeroizing<String>,
    public_key: String,
}

#[derive(Deserialize)]
struct Response {
    binding: nyxid_machine::managed::Binding,
    sealed: sealed_bootstrap::Envelope,
    server_url: String,
    writer_admitted: bool,
}

#[derive(Deserialize)]
struct Secrets {
    auth_token: Zeroizing<String>,
    signing_secret: Zeroizing<String>,
    #[serde(default = "default_capabilities")]
    capabilities: Vec<String>,
}

const MAX_BOOTSTRAP_BYTES: usize = 64 * 1024;

#[derive(Serialize, Deserialize)]
struct KubernetesRecipient {
    node_id: String,
    pod_uid: String,
    private_key: Zeroizing<String>,
    public_key: String,
}

#[derive(Serialize)]
struct KubernetesTicket<'a> {
    token: &'a str,
    recipient_key_digest: &'a str,
}

#[derive(Serialize)]
struct KubernetesPrepareRequest<'a> {
    node_id: &'a str,
    pod_uid: &'a str,
    ticket: &'a KubernetesTicket<'a>,
    recipient_public_key: &'a str,
}

#[derive(Serialize)]
struct KubernetesBootstrapRequest<'a> {
    binding: &'a nyxid_machine::managed::Binding,
    ticket: &'a KubernetesTicket<'a>,
    recipient_public_key: &'a str,
}

#[derive(Clone)]
struct KubernetesSetup {
    node_id: String,
    pod_uid: String,
    token_file: PathBuf,
    prepare_url: Url,
    bootstrap_url: Url,
}

async fn bootstrap_kubernetes(args: &Start) -> Result<PathBuf> {
    let env = |name: &str| {
        std::env::var(name).with_context(|| format!("{name} is required for Kubernetes bootstrap"))
    };
    let setup = KubernetesSetup {
        node_id: env("NYXID_MANAGED_NODE_ID")?,
        pod_uid: env("NYXID_MANAGED_POD_UID")?,
        token_file: PathBuf::from(env("NYXID_MANAGED_WORKLOAD_TOKEN_FILE")?),
        prepare_url: validate_endpoint(&env("NYXID_MANAGED_PREPARE_URL")?, false)?,
        bootstrap_url: validate_endpoint(&args.bootstrap_url, false)?,
    };
    ensure!(
        uuid::Uuid::parse_str(&setup.node_id).is_ok()
            && uuid::Uuid::parse_str(&setup.pod_uid).is_ok(),
        "invalid Kubernetes workload identity"
    );
    ensure!(
        setup.prepare_url.origin() == setup.bootstrap_url.origin(),
        "managed prepare and bootstrap origins must match"
    );
    let directory = config::resolve_config_dir(args.config.as_deref());
    bootstrap_kubernetes_with_setup(&setup, &directory, Duration::from_secs(300)).await
}

fn kubernetes_recipient(setup: &KubernetesSetup, directory: &Path) -> Result<KubernetesRecipient> {
    let path = directory.join("managed-kubernetes-recipient.json");
    if path.exists() {
        let recipient: KubernetesRecipient = serde_json::from_slice(&read_private_file(&path)?)?;
        ensure!(
            recipient.node_id == setup.node_id && recipient.pod_uid == setup.pod_uid,
            "pending Kubernetes recipient belongs to another workload"
        );
        validate_recipient(&recipient.private_key, &recipient.public_key)?;
        return Ok(recipient);
    }
    let pair = sealed_bootstrap::generate_recipient()?;
    let recipient = KubernetesRecipient {
        node_id: setup.node_id.clone(),
        pod_uid: setup.pod_uid.clone(),
        private_key: sealed_bootstrap::encode_private_key(&pair)?,
        public_key: pair.public_key_b64u()?,
    };
    atomic_private_json(&path, &recipient)?;
    Ok(recipient)
}

fn read_workload_token(path: &Path) -> Result<Zeroizing<String>> {
    use std::os::unix::fs::MetadataExt;
    // Kubernetes projected files use a root-owned symlink into the atomic
    // ..data directory. Check the opened target, rather than banning that
    // established projection mechanism or accepting a world-readable token.
    let mut file = std::fs::File::open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0
            && metadata.len() <= MAX_BOOTSTRAP_BYTES as u64,
        "workload identity token must be a protected bounded file"
    );
    let mut token = Zeroizing::new(String::new());
    (&mut file)
        .take((MAX_BOOTSTRAP_BYTES + 1) as u64)
        .read_to_string(&mut token)?;
    ensure!(
        !token.trim().is_empty() && token.len() <= MAX_BOOTSTRAP_BYTES,
        "workload identity token is empty or too large"
    );
    Ok(Zeroizing::new(token.trim().to_owned()))
}

async fn bounded_json_response(mut response: reqwest::Response) -> Result<Zeroizing<Vec<u8>>> {
    ensure!(
        response.status().is_success(),
        "managed workload response refused"
    );
    let mut bytes = Zeroizing::new(Vec::new());
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len().saturating_add(chunk.len()) <= MAX_BOOTSTRAP_BYTES,
            "managed response is too large"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn bootstrap_kubernetes_with_setup(
    setup: &KubernetesSetup,
    directory: &Path,
    budget: Duration,
) -> Result<PathBuf> {
    prepare_directory(directory)?;
    let recipient_setup = setup.clone();
    let recipient_directory = directory.to_owned();
    let recipient = tokio::task::spawn_blocking(move || {
        kubernetes_recipient(&recipient_setup, &recipient_directory)
    })
    .await
    .context("prepare managed recipient")??;
    let digest = hex::encode(sealed_bootstrap::recipient_key_digest(
        &recipient.public_key,
    )?);
    let client = crate::tls::client_builder()?
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let deadline = tokio::time::Instant::now() + budget;
    let mut backoff = Duration::from_secs(1);
    loop {
        let attempt = async {
            let token = read_workload_token(&setup.token_file)?;
            let ticket = KubernetesTicket {
                token: token.as_str(),
                recipient_key_digest: &digest,
            };
            let prepare = client
                .post(setup.prepare_url.clone())
                .json(&KubernetesPrepareRequest {
                    node_id: &setup.node_id,
                    pod_uid: &setup.pod_uid,
                    ticket: &ticket,
                    recipient_public_key: &recipient.public_key,
                })
                .send()
                .await?;
            // Retry only transient convergence and transport responses; a
            // source denial or unsafe redirect is not an invitation to retry.
            if matches!(
                prepare.status().as_u16(),
                409 | 425 | 429 | 500 | 502 | 503 | 504
            ) {
                return Ok::<_, anyhow::Error>(None);
            }
            let value: Value = serde_json::from_slice(&bounded_json_response(prepare).await?)?;
            let binding: nyxid_machine::managed::Binding =
                serde_json::from_value(value["binding"].clone())?;
            binding.validate().map_err(anyhow::Error::msg)?;
            ensure!(
                binding.node_id == setup.node_id && binding.pod_uid == setup.pod_uid,
                "prepared workload target mismatch"
            );
            if directory.join("config.toml").exists() {
                let current = NodeConfig::load(&directory.join("config.toml"))?;
                let previous = current
                    .managed_runtime
                    .as_ref()
                    .context("existing identity is not managed")?;
                if previous == &binding {
                    validate_existing_config(&current, &binding)?;
                    return Ok(Some(directory.to_owned()));
                }
                ensure!(
                    previous.node_id == binding.node_id
                        && (binding.runtime_generation > previous.runtime_generation
                            || (binding.runtime_generation == previous.runtime_generation
                                && binding.credential_epoch > previous.credential_epoch)),
                    "managed rekey must advance the current workload fence"
                );
            }
            let pending = PendingDelivery {
                binding: binding.clone(),
                private_key: recipient.private_key.clone(),
                public_key: recipient.public_key.clone(),
            };
            atomic_private_json(&directory.join("managed-bootstrap.json"), &pending)?;
            let response = client
                .post(setup.bootstrap_url.clone())
                .json(&KubernetesBootstrapRequest {
                    binding: &binding,
                    ticket: &ticket,
                    recipient_public_key: &recipient.public_key,
                })
                .send()
                .await?;
            if matches!(
                response.status().as_u16(),
                409 | 425 | 429 | 500 | 502 | 503 | 504
            ) {
                return Ok(None);
            }
            let response: Response =
                serde_json::from_slice(&bounded_json_response(response).await?)?;
            install_delivery(directory.to_owned(), &pending, response).map(Some)
        };
        match tokio::time::timeout_at(deadline, attempt).await {
            Ok(Ok(Some(path))) => return Ok(path),
            Ok(Ok(None)) => {}
            Ok(Err(error))
                if error
                    .downcast_ref::<reqwest::Error>()
                    .is_some_and(|error| error.is_connect() || error.is_timeout()) => {}
            Ok(Err(error)) => return Err(error.context("managed Kubernetes bootstrap refused")),
            Err(_) => anyhow::bail!(
                "managed Kubernetes bootstrap deadline exceeded; inspect Machine operation status"
            ),
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        ensure!(
            remaining > backoff,
            "managed Kubernetes bootstrap deadline exceeded; inspect Machine operation status"
        );
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(15));
    }
}

pub fn require_self_hosted_mutation(directory: &Path) -> Result<()> {
    let path = directory.join("config.toml");
    if path.exists() {
        ensure!(
            NodeConfig::load(&path)?.managed_runtime.is_none(),
            "managed node identity and profile changes require the Remote Machine lifecycle manager"
        );
    }
    ensure!(
        !directory.join("managed-bootstrap.json").exists(),
        "managed bootstrap is pending; ordinary registration or profile changes are not permitted"
    );
    Ok(())
}

pub async fn prepare(args: Prepare) -> Result<()> {
    let directory = config::resolve_config_dir(args.config.as_deref());
    prepare_directory(&directory)?;
    ensure!(
        !directory.join("config.toml").exists(),
        "managed credentials already activated; use controlled platform rekey"
    );
    let bytes = read_bounded_file(&args.ticket_file)?;
    let ticket: Ticket =
        serde_json::from_slice(&bytes).context("decode managed bootstrap ticket")?;
    ticket.binding.validate().map_err(anyhow::Error::msg)?;
    let recipient_directory = directory.clone();
    let pending = tokio::task::spawn_blocking(move || {
        prepare_delivery(&recipient_directory, &ticket.binding)
    })
    .await
    .context("prepare managed delivery")??;
    println!("{}", serde_json::to_string(&recipient_request(&pending)?)?);
    Ok(())
}

pub async fn run(args: Start) -> Result<()> {
    let config_dir = bootstrap_config(&args).await?;
    let config = NodeConfig::load(&config_dir.join("config.toml"))?;
    if config.machine.browser_enabled() || config.machine.computer {
        // The full image and self-hosted setup share exactly one desktop
        // implementation. No user-controlled command or capability flags are
        // accepted here: these choices came from the sealed bootstrap.
        let status = tokio::process::Command::new("/usr/local/bin/nyxid-machine-desktop-start")
            .status()
            .await
            .context("start managed Machine desktop")?;
        ensure!(status.success(), "managed Machine desktop startup failed");
    }
    super::agent::cmd_start(
        Some(config_dir.to_str().context("invalid config path")?),
        None,
    )
    .await
    .map_err(anyhow::Error::from)
}

async fn bootstrap_config(args: &Start) -> Result<PathBuf> {
    if args.kubernetes {
        return bootstrap_kubernetes(args).await;
    }
    let config_dir = config::resolve_config_dir(args.config.as_deref());
    prepare_directory(&config_dir)?;
    let config_path = config_dir.join("config.toml");
    let ticket_bytes = read_bounded_file(
        args.ticket_file
            .as_deref()
            .context("managed ticket file required")?,
    )
    .context("read managed bootstrap ticket")?;
    let ticket: Ticket =
        serde_json::from_slice(&ticket_bytes).context("decode managed bootstrap ticket")?;
    ticket.binding.validate().map_err(anyhow::Error::msg)?;
    if config_path.exists() {
        let config = NodeConfig::load(&config_path)?;
        validate_existing_config(&config, &ticket.binding)?;
        return Ok(config_dir);
    }
    let bootstrap_url = validate_endpoint(&args.bootstrap_url, false)?;
    let recipient_directory = config_dir.clone();
    let recipient_binding = ticket.binding.clone();
    let pending = tokio::task::spawn_blocking(move || {
        prepare_delivery(&recipient_directory, &recipient_binding)
    })
    .await
    .context("prepare managed delivery")??;
    ensure!(
        ticket.token.as_ref().is_some_and(|token| !token.is_empty()),
        "managed provider ticket pending: sign the recipient-bound managed-bootstrap-request.json and refill the protected ticket file"
    );

    let client = crate::tls::client_builder()?
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut response = client
        .post(bootstrap_url)
        .json(&serde_json::json!({
            "ticket": serde_json::from_slice::<Value>(&ticket_bytes)?,
            "binding": pending.binding.clone(),
            "recipient_public_key": pending.public_key,
        }))
        .send()
        .await?
        .error_for_status()
        .context("managed bootstrap rejected")?;
    ensure!(
        response.status().is_success(),
        "managed bootstrap redirect rejected"
    );
    let mut body = Zeroizing::new(Vec::new());
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            body.len() + chunk.len() <= MAX_BOOTSTRAP_BYTES,
            "managed bootstrap response is too large"
        );
        body.extend_from_slice(&chunk);
    }
    let response: Response =
        serde_json::from_slice(&body).context("decode managed bootstrap response")?;
    ensure!(
        response.writer_admitted,
        "managed runtime writer admission is not proven"
    );
    ensure!(
        response.binding == ticket.binding,
        "managed bootstrap binding mismatch"
    );
    install_delivery(config_dir, &pending, response)
}

fn install_delivery(
    config_dir: PathBuf,
    pending: &PendingDelivery,
    response: Response,
) -> Result<PathBuf> {
    ensure!(
        response.writer_admitted,
        "managed runtime writer admission is not proven"
    );
    ensure!(
        response.binding == pending.binding,
        "managed bootstrap binding mismatch"
    );
    validate_endpoint(&response.server_url, true)?;
    let private_key = sealed_bootstrap::decode_private_key(&pending.private_key)
        .context("managed bootstrap recipient is incompatible; controlled rekey required")?;
    let plaintext = sealed_bootstrap::open(&response.sealed, &private_key, &pending.binding)
        .context("managed bootstrap decryption failed")?;
    let secrets: Secrets =
        serde_json::from_slice(&plaintext).context("decode managed bootstrap secrets")?;
    ensure!(
        !secrets.auth_token.is_empty() && !secrets.signing_secret.is_empty(),
        "managed bootstrap returned empty credentials"
    );

    let backend = SecretBackend::new("file", &pending.binding.node_id, &config_dir)?;
    let mut config = NodeConfig::new(
        response.server_url,
        pending.binding.node_id.clone(),
        "file".into(),
    );
    config.managed_runtime = Some(pending.binding.clone());
    config.machine = machine_config(&secrets.capabilities)?;
    let mut capabilities = secrets.capabilities;
    capabilities.sort();
    config.managed_capabilities = Some(capabilities);
    backend.store_auth_token(&mut config, &secrets.auth_token)?;
    backend.store_signing_secret(&mut config, &secrets.signing_secret)?;
    config.save(&config_dir.join("config.toml"))?;
    std::fs::remove_file(config_dir.join("managed-bootstrap.json")).ok();
    std::fs::remove_file(config_dir.join("managed-bootstrap-request.json")).ok();
    std::fs::remove_file(config_dir.join("managed-kubernetes-recipient.json")).ok();
    Ok(config_dir)
}

/// Called only after WSS acknowledges the exact live runtime fence. Even a
/// successfully delivered credential is not sufficient to mutate workspace
/// ownership if a pause or rekey raced with the subsequent connection.
pub(super) fn admit_workspace() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        // Mounted homes/PVCs may retain ownership or a permissive mount mode.
        // CHOWN permits restoring the fixed modes without DAC_OVERRIDE/FOWNER.
        let status = std::process::Command::new("chown")
            .args(["root:root", "/workspace", "/home/agent"])
            .status()?;
        ensure!(status.success(), "managed workspace admission setup failed");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions("/workspace", std::fs::Permissions::from_mode(0o755))?;
        std::fs::set_permissions("/home/agent", std::fs::Permissions::from_mode(0o700))?;
        let status = std::process::Command::new("chown")
            .args(["agent:agent", "/workspace", "/home/agent"])
            .status()?;
        ensure!(status.success(), "managed workspace admission setup failed");
    }
    Ok(())
}

/// End the whole managed supervisor when its protected execution window is
/// terminal. Runtime::shutdown stops jobs/drivers; leaving the Node alive would
/// keep the image's desktop sessions alive indefinitely after lease expiry.
pub(super) async fn wait_for_runtime_end(
    directory: &Path,
    binding: Option<&nyxid_machine::managed::Binding>,
) {
    let Some(binding) = binding else {
        std::future::pending::<()>().await;
        return;
    };
    let started = tokio::time::Instant::now();
    let mut admitted = false;
    let mut interval = tokio::time::interval(Duration::from_millis(100));
    loop {
        interval.tick().await;
        let result = read_private_file(&directory.join("managed-deadline-v1.json"));
        match result {
            Ok(bytes) => {
                let value: Value = match serde_json::from_slice(&bytes) {
                    Ok(value) => value,
                    Err(_) => return,
                };
                if value.get("binding") != serde_json::to_value(binding).ok().as_ref() {
                    if admitted {
                        return;
                    }
                } else {
                    admitted = true;
                    let mut marker: super::managed_deadline::DeadlineMarker =
                        match serde_json::from_value(value) {
                            Ok(marker) => marker,
                            Err(_) => return,
                        };
                    if !marker.live() {
                        let _ = super::managed_deadline::close_deadline_marker(directory, &marker);
                        return;
                    }
                }
            }
            Err(error)
                if !admitted
                    && error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) => {}
            Err(_) => return,
        }
        // Credential delivery without successful WSS admission must not keep
        // even an idle desktop alive indefinitely.
        if !admitted && started.elapsed() >= Duration::from_secs(300) {
            return;
        }
    }
}

fn read_private_file(path: &Path) -> Result<Zeroizing<Vec<u8>>> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0
            && metadata.len() <= MAX_BOOTSTRAP_BYTES as u64,
        "managed recipient must be a protected bounded file"
    );
    let mut bytes = Zeroizing::new(Vec::new());
    (&mut file)
        .take((MAX_BOOTSTRAP_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_BOOTSTRAP_BYTES,
        "managed recipient file is too large"
    );
    Ok(bytes)
}

fn read_bounded_file(path: &Path) -> Result<Zeroizing<Vec<u8>>> {
    let mut bytes = Zeroizing::new(Vec::new());
    std::fs::File::open(path)?
        .take((MAX_BOOTSTRAP_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_BOOTSTRAP_BYTES,
        "managed bootstrap file is too large"
    );
    Ok(bytes)
}

fn prepare_directory(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)?;
    ensure!(
        !std::fs::symlink_metadata(path)?.file_type().is_symlink(),
        "managed identity directory must not be a symlink"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn validate_recipient(private_key: &str, public_key: &str) -> Result<()> {
    let recipient = sealed_bootstrap::decode_private_key(private_key)
        .context("managed bootstrap recipient is incompatible; controlled rekey required")?;
    ensure!(
        recipient.public_key_b64u()? == public_key,
        "managed bootstrap recipient key mismatch; controlled rekey required"
    );
    Ok(())
}

fn recipient_request(pending: &PendingDelivery) -> Result<Value> {
    let digest = sealed_bootstrap::recipient_key_digest(&pending.public_key)?;
    Ok(serde_json::json!({
        "binding": pending.binding,
        "recipient_public_key": pending.public_key,
        "recipient_key_digest": hex::encode(digest),
    }))
}

fn prepare_delivery(
    directory: &Path,
    binding: &nyxid_machine::managed::Binding,
) -> Result<PendingDelivery> {
    let path = directory.join("managed-bootstrap.json");
    let pending = if path.exists() {
        let pending: PendingDelivery = serde_json::from_slice(&read_private_file(&path)?)
            .context("decode pending managed delivery")?;
        ensure!(
            &pending.binding == binding,
            "pending managed delivery binding changed"
        );
        validate_recipient(&pending.private_key, &pending.public_key)?;
        pending
    } else {
        let keypair = sealed_bootstrap::generate_recipient()?;
        let pending = PendingDelivery {
            binding: binding.clone(),
            private_key: sealed_bootstrap::encode_private_key(&keypair)?,
            public_key: keypair.public_key_b64u()?,
        };
        atomic_private_json(&path, &pending)?;
        pending
    };
    atomic_private_json(
        &directory.join("managed-bootstrap-request.json"),
        &recipient_request(&pending)?,
    )?;
    Ok(pending)
}

fn atomic_private_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = Zeroizing::new(serde_json::to_vec(value)?);
    use std::io::Write;
    let directory = path
        .parent()
        .context("managed delivery path has no parent")?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    #[cfg(unix)]
    std::fs::File::open(directory)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn binding() -> nyxid_machine::managed::Binding {
        nyxid_machine::managed::Binding {
            node_id: uuid::Uuid::new_v4().to_string(),
            sandbox_uid: uuid::Uuid::new_v4().to_string(),
            pod_uid: uuid::Uuid::new_v4().to_string(),
            runtime_generation: 1,
            credential_epoch: 1,
            writer_fence: uuid::Uuid::new_v4().to_string(),
            operation_id: uuid::Uuid::new_v4().to_string(),
            delivery_id: uuid::Uuid::new_v4().to_string(),
        }
    }

    fn invalid_envelope() -> sealed_bootstrap::Envelope {
        use base64::Engine;
        use rand::RngCore;
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let mut nonce = [0; 12];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        serde_json::from_value(serde_json::json!({
            "version": 1,
            "wrapped_key": b64.encode([0; 384]),
            "nonce": b64.encode(nonce),
            "ciphertext": b64.encode([0; 16]),
        }))
        .unwrap()
    }

    fn protected_token(path: &Path, token: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, token).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    #[tokio::test]
    async fn kubernetes_bootstrap_retries_same_recipient_refreshes_projection_and_installs_sealed_identity()
     {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let directory = tempfile::tempdir().unwrap();
        let node = directory.path().join("node");
        let token_path = directory.path().join("token");
        protected_token(&token_path, "projected-fixture-token-one");
        let binding = binding();
        let server = MockServer::start().await;
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_prepare = calls.clone();
        let expected = binding.clone();
        let token_for_prepare = token_path.clone();
        Mock::given(method("POST"))
            .and(path("/prepare"))
            .respond_with(move |request: &wiremock::Request| {
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                assert_eq!(body["node_id"], expected.node_id);
                assert_eq!(body["pod_uid"], expected.pod_uid);
                let digest = sealed_bootstrap::recipient_key_digest(
                    body["recipient_public_key"].as_str().unwrap(),
                )
                .unwrap();
                assert_eq!(body["ticket"]["recipient_key_digest"], hex::encode(digest));
                if calls_for_prepare.fetch_add(1, Ordering::SeqCst) == 0 {
                    assert_eq!(body["ticket"]["token"], "projected-fixture-token-one");
                    protected_token(&token_for_prepare, "projected-fixture-token-two");
                    ResponseTemplate::new(503)
                } else {
                    assert_eq!(body["ticket"]["token"], "projected-fixture-token-two");
                    ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({"binding":expected}))
                }
            })
            .expect(2)
            .mount(&server)
            .await;
        let expected = binding.clone();
        Mock::given(method("POST")).and(path("/bootstrap")).respond_with(move |request: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            assert_eq!(body["ticket"]["token"], "projected-fixture-token-two");
            let public_key = body["recipient_public_key"].as_str().unwrap();
            let payload = serde_json::to_vec(&serde_json::json!({"auth_token":"nyx_nauth_kubernetes_fixture","signing_secret":"b".repeat(64)})).unwrap();
            let sealed = sealed_bootstrap::seal(&payload, public_key, &expected).unwrap();
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"binding":expected,"sealed":sealed,"server_url":"wss://example.test/api/v1/nodes/ws","writer_admitted":true}))
        }).expect(1).mount(&server).await;
        let setup = KubernetesSetup {
            node_id: binding.node_id.clone(),
            pod_uid: binding.pod_uid.clone(),
            token_file: token_path,
            prepare_url: Url::parse(&format!("{}/prepare", server.uri())).unwrap(),
            bootstrap_url: Url::parse(&format!("{}/bootstrap", server.uri())).unwrap(),
        };
        assert_eq!(
            bootstrap_kubernetes_with_setup(&setup, &node, Duration::from_secs(10))
                .await
                .unwrap(),
            node
        );
        validate_existing_config(
            &NodeConfig::load(&node.join("config.toml")).unwrap(),
            &binding,
        )
        .unwrap();
        let requests = server.received_requests().await.unwrap();
        let keys: Vec<_> = requests
            .iter()
            .map(|request| {
                serde_json::from_slice::<Value>(&request.body).unwrap()["recipient_public_key"]
                    .clone()
            })
            .collect();
        assert!(keys.iter().all(|key| *key == keys[0]));
        assert!(!node.join("managed-kubernetes-recipient.json").exists());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn kubernetes_rekey_retains_old_encrypted_config_until_sealed_new_epoch_is_verified() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let directory = tempfile::tempdir().unwrap();
        let node = directory.path().join("node");
        prepare_directory(&node).unwrap();
        let token_path = directory.path().join("token");
        protected_token(&token_path, "fixture-projected-identity");
        let previous = binding();
        managed_config(&previous)
            .save(&node.join("config.toml"))
            .unwrap();
        let before = std::fs::read(node.join("config.toml")).unwrap();
        let mut next = previous.clone();
        next.credential_epoch += 1;
        next.delivery_id = uuid::Uuid::new_v4().to_string();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/prepare"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"binding":next})),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST")).and(path("/bootstrap")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"binding":next,"sealed":invalid_envelope(),"server_url":"wss://example.test/api/v1/nodes/ws","writer_admitted":true}))).mount(&server).await;
        let setup = KubernetesSetup {
            node_id: previous.node_id,
            pod_uid: previous.pod_uid,
            token_file: token_path,
            prepare_url: Url::parse(&format!("{}/prepare", server.uri())).unwrap(),
            bootstrap_url: Url::parse(&format!("{}/bootstrap", server.uri())).unwrap(),
        };
        assert!(
            bootstrap_kubernetes_with_setup(&setup, &node, Duration::from_secs(5))
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(node.join("config.toml")).unwrap(), before);
        let first: KubernetesRecipient = serde_json::from_slice(
            &read_bounded_file(&node.join("managed-kubernetes-recipient.json")).unwrap(),
        )
        .unwrap();
        let replay = kubernetes_recipient(&setup, &node).unwrap();
        assert_eq!(first.public_key, replay.public_key);
    }

    #[test]
    fn projected_identity_must_be_private_and_never_accepts_an_empty_or_oversized_token() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let token = directory.path().join("token");
        protected_token(&token, "fixture");
        assert!(read_workload_token(&token).is_ok());
        std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_workload_token(&token).is_err());
        protected_token(&token, " ");
        assert!(read_workload_token(&token).is_err());
        protected_token(&token, &"x".repeat(MAX_BOOTSTRAP_BYTES + 1));
        assert!(read_workload_token(&token).is_err());
    }

    fn managed_config(binding: &nyxid_machine::managed::Binding) -> NodeConfig {
        let mut config = NodeConfig::new(
            "wss://nyxid.example.test/api/v1/nodes/ws".into(),
            binding.node_id.clone(),
            "file".into(),
        );
        config.managed_runtime = Some(binding.clone());
        config.machine.shell = true;
        config.machine.files = true;
        config.machine.browser = Some(false);
        config.machine.roots = vec![PathBuf::from("/workspace")];
        config.machine.max_jobs = 2;
        config.machine.agent_user = Some("agent".into());
        config.machine.browser_user = Some("browser".into());
        config
    }

    #[test]
    fn restart_requires_exact_persisted_managed_binding() {
        let binding = binding();
        let mut config = managed_config(&binding);
        assert!(validate_existing_config(&config, &binding).is_ok());
        config.managed_runtime = None;
        assert!(validate_existing_config(&config, &binding).is_err());
        config.managed_runtime = Some(binding.clone());
        config.managed_runtime.as_mut().unwrap().credential_epoch += 1;
        assert!(validate_existing_config(&config, &binding).is_err());
        config.managed_runtime = Some(binding.clone());
        config.node.id = uuid::Uuid::new_v4().to_string();
        assert!(validate_existing_config(&config, &binding).is_err());
    }

    #[test]
    fn restart_rejects_profile_drift() {
        let binding = binding();
        let mut config = managed_config(&binding);
        config.machine.computer = true;
        assert!(validate_existing_config(&config, &binding).is_err());
        config.machine.computer = false;
        config.machine.max_jobs = 3;
        assert!(validate_existing_config(&config, &binding).is_err());
        config.machine.max_jobs = 2;
        config
            .machine
            .roots
            .push(PathBuf::from("/var/lib/nyxid-machine"));
        assert!(validate_existing_config(&config, &binding).is_err());
    }

    #[test]
    fn sealed_capabilities_select_full_container_runtime_and_survive_reload() {
        let binding = binding();
        let mut config = managed_config(&binding);
        let capabilities = vec!["browser".into(), "computer".into(), "files".into()];
        config.machine = machine_config(&capabilities).unwrap();
        config.managed_capabilities = Some(capabilities);
        assert!(!config.machine.shell);
        assert!(
            config.machine.files && config.machine.browser_enabled() && config.machine.computer
        );
        assert_eq!(
            config.machine.cua_driver,
            Some(PathBuf::from("/opt/nyxid/cua/cua-driver"))
        );
        assert_eq!(
            config.machine.effective_dev_browser_user(),
            Some("devbrowser")
        );
        assert_eq!(
            config.machine.managed_browser.as_ref().unwrap().binary,
            PathBuf::from("/usr/bin/chromium")
        );
        assert!(validate_existing_config(&config, &binding).is_ok());
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        config.save(&path).unwrap();
        assert_eq!(
            NodeConfig::load(&path).unwrap().managed_capabilities,
            config.managed_capabilities
        );
        config.machine.shell = true;
        config.save(&path).unwrap();
        assert!(
            NodeConfig::load(&path).is_err(),
            "a local config reload cannot widen the sealed ceiling"
        );
    }

    #[test]
    fn sealed_capabilities_reject_empty_unknown_duplicate_and_computer_without_browser() {
        for values in [
            vec![],
            vec!["unknown"],
            vec!["shell", "shell"],
            vec!["computer"],
        ] {
            let values: Vec<String> = values.into_iter().map(str::to_owned).collect();
            assert!(machine_config(&values).is_err());
        }
        assert!(
            machine_config(&["browser".into()])
                .unwrap()
                .browser_enabled()
        );
        assert!(machine_config(&["files".into()]).unwrap().files);
        let legacy: Secrets = serde_json::from_value(
            serde_json::json!({"auth_token":"fixture", "signing_secret":"fixture"}),
        )
        .unwrap();
        assert_eq!(legacy.capabilities, default_capabilities());
    }

    #[tokio::test]
    async fn managed_supervisor_ends_when_the_protected_window_closes() {
        let directory = tempfile::tempdir().unwrap();
        let binding = binding();
        let now = chrono::Utc::now().timestamp_millis();
        let window = nyxid_machine::managed::RuntimeWindow {
            binding: binding.clone(),
            control_revision: 1,
            run_not_after_ms: now + 60_000,
            issued_at_ms: now,
        };
        let marker =
            super::super::managed_deadline::install_deadline_marker(directory.path(), &window)
                .unwrap();
        assert!(
            tokio::time::timeout(
                Duration::from_millis(150),
                wait_for_runtime_end(directory.path(), Some(&binding))
            )
            .await
            .is_err()
        );
        super::super::managed_deadline::close_deadline_marker(directory.path(), &marker).unwrap();
        tokio::time::timeout(
            Duration::from_secs(1),
            wait_for_runtime_end(directory.path(), Some(&binding)),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn bootstrap_retry_keeps_recipient_and_stale_ticket_cannot_reuse_delivery() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let directory = tempfile::tempdir().unwrap();
        let config_dir = directory.path().join("node");
        let ticket_file = directory.path().join("ticket.json");
        let mut binding = binding();
        std::fs::write(
            &ticket_file,
            serde_json::to_vec(&serde_json::json!({"binding":binding,"token":"fixture-token"}))
                .unwrap(),
        )
        .unwrap();
        let server = MockServer::start().await;
        // A correctly shaped but non-admitted response must be rejected before
        // decryption, credential persistence or command runtime startup.
        let sealed = invalid_envelope();
        Mock::given(method("POST")).and(path("/bootstrap"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "binding":binding,"sealed":sealed,"server_url":"wss://example.test/api/v1/nodes/ws","writer_admitted":false
            }))).expect(2).mount(&server).await;
        let args = || Start {
            ticket_file: Some(ticket_file.clone()),
            kubernetes: false,
            bootstrap_url: format!("{}/bootstrap", server.uri()),
            config: Some(config_dir.to_str().unwrap().into()),
        };
        for _ in 0..2 {
            let error = run(args()).await.unwrap_err();
            assert!(error.to_string().contains("writer admission is not proven"));
            assert!(!config_dir.join("config.toml").exists());
        }
        let requests = server.received_requests().await.unwrap();
        let first: Value = serde_json::from_slice(&requests[0].body).unwrap();
        let second: Value = serde_json::from_slice(&requests[1].body).unwrap();
        assert_eq!(
            first["recipient_public_key"],
            second["recipient_public_key"]
        );
        assert_eq!(
            first["binding"]["delivery_id"],
            second["binding"]["delivery_id"]
        );
        assert!(first.get("auth_token").is_none() && first.get("signing_secret").is_none());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(config_dir.join("managed-bootstrap.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        binding.credential_epoch += 1;
        std::fs::write(
            &ticket_file,
            serde_json::to_vec(&serde_json::json!({"binding":binding})).unwrap(),
        )
        .unwrap();
        assert!(
            run(args())
                .await
                .unwrap_err()
                .to_string()
                .contains("pending managed delivery binding changed")
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }

    #[test]
    fn pending_delivery_round_trip_contains_no_plaintext_credential() {
        let pending = PendingDelivery {
            binding: binding(),
            private_key: Zeroizing::new("private-key".into()),
            public_key: "public-key".into(),
        };
        let encoded = serde_json::to_string(&pending).unwrap();
        assert!(!encoded.contains("auth_token") && !encoded.contains("signing_secret"));
    }

    #[test]
    fn managed_endpoints_require_safe_transport_and_node_route() {
        for bad in [
            "http://example.test/bootstrap",
            "https://user:secret@example.test/bootstrap",
            "https://example.test/bootstrap?ticket=secret",
            "https://example.test/bootstrap#token",
        ] {
            assert!(validate_endpoint(bad, false).is_err(), "{bad}");
        }
        for bad in [
            "https://example.test/api/v1/nodes/ws",
            "ws://example.test/api/v1/nodes/ws",
            "wss://example.test/",
            "wss://example.test/api/v1/nodes/ws?token=secret",
        ] {
            assert!(validate_endpoint(bad, true).is_err(), "{bad}");
        }
        assert!(
            validate_endpoint(
                "https://example.test/api/v1/remote-machine-bootstrap",
                false
            )
            .is_ok()
        );
        assert!(validate_endpoint("ws://127.0.0.1:3001/api/v1/nodes/ws", true).is_ok());
        assert!(validate_endpoint("wss://example.test/api/v1/nodes/ws", true).is_ok());
    }

    #[tokio::test]
    async fn unsigned_ticket_prepares_recipient_without_network_or_credentials() {
        use wiremock::MockServer;
        let directory = tempfile::tempdir().unwrap();
        let config_dir = directory.path().join("node");
        let ticket_file = directory.path().join("ticket.json");
        let binding = binding();
        std::fs::write(
            &ticket_file,
            serde_json::to_vec(&serde_json::json!({"binding":binding})).unwrap(),
        )
        .unwrap();
        let server = MockServer::start().await;
        for _ in 0..2 {
            let error = run(Start {
                ticket_file: Some(ticket_file.clone()),
                kubernetes: false,
                bootstrap_url: format!("{}/bootstrap", server.uri()),
                config: Some(config_dir.to_str().unwrap().into()),
            })
            .await
            .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("managed provider ticket pending")
            );
        }
        assert!(server.received_requests().await.unwrap().is_empty());
        assert!(!config_dir.join("config.toml").exists());
        let pending: PendingDelivery = serde_json::from_slice(
            &std::fs::read(config_dir.join("managed-bootstrap.json")).unwrap(),
        )
        .unwrap();
        let request: Value = serde_json::from_slice(
            &std::fs::read(config_dir.join("managed-bootstrap-request.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(request, recipient_request(&pending).unwrap());
        assert_eq!(request["binding"], serde_json::to_value(&binding).unwrap());
        assert!(request.get("private_key").is_none());
        let digest = sealed_bootstrap::recipient_key_digest(&pending.public_key).unwrap();
        assert_eq!(request["recipient_key_digest"], hex::encode(digest));
    }

    #[tokio::test]
    async fn bootstrap_does_not_forward_ticket_through_redirect() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let directory = tempfile::tempdir().unwrap();
        let ticket_file = directory.path().join("ticket.json");
        std::fs::write(
            &ticket_file,
            serde_json::to_vec(
                &serde_json::json!({"binding":binding(), "token":"provider-secret-ticket"}),
            )
            .unwrap(),
        )
        .unwrap();
        let server = MockServer::start().await;
        let receiver = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bootstrap"))
            .respond_with(
                ResponseTemplate::new(307)
                    .insert_header("Location", format!("{}/leak", receiver.uri())),
            )
            .expect(1)
            .mount(&server)
            .await;
        let error = run(Start {
            ticket_file: Some(ticket_file),
            kubernetes: false,
            bootstrap_url: format!("{}/bootstrap", server.uri()),
            config: Some(directory.path().join("node").to_str().unwrap().into()),
        })
        .await
        .unwrap_err();
        assert!(error.to_string().contains("redirect rejected"));
        assert!(receiver.received_requests().await.unwrap().is_empty());
    }

    #[test]
    fn incompatible_pending_recipients_are_preserved_and_require_controlled_rekey() {
        let directory = tempfile::tempdir().unwrap();
        prepare_directory(directory.path()).unwrap();
        let binding = binding();
        // Pre-MVP recipients used a raw 32-byte key. Never silently replace
        // that key after a server may already have activated its delivery.
        let obsolete_key = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let pending = PendingDelivery {
            binding: binding.clone(),
            private_key: Zeroizing::new(obsolete_key.into()),
            public_key: obsolete_key.into(),
        };
        let path = directory.path().join("managed-bootstrap.json");
        atomic_private_json(&path, &pending).unwrap();
        let before = std::fs::read(&path).unwrap();
        let error = prepare_delivery(directory.path(), &binding).err().unwrap();
        assert!(error.to_string().contains("controlled rekey required"));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(
            !directory
                .path()
                .join("managed-bootstrap-request.json")
                .exists()
        );

        let recipient = KubernetesRecipient {
            node_id: binding.node_id.clone(),
            pod_uid: binding.pod_uid.clone(),
            private_key: Zeroizing::new(obsolete_key.into()),
            public_key: obsolete_key.into(),
        };
        let path = directory.path().join("managed-kubernetes-recipient.json");
        atomic_private_json(&path, &recipient).unwrap();
        let before = std::fs::read(&path).unwrap();
        let setup = KubernetesSetup {
            node_id: binding.node_id,
            pod_uid: binding.pod_uid,
            token_file: directory.path().join("unused-token"),
            prepare_url: Url::parse("https://example.test/prepare").unwrap(),
            bootstrap_url: Url::parse("https://example.test/bootstrap").unwrap(),
        };
        let error = kubernetes_recipient(&setup, directory.path())
            .err()
            .unwrap();
        assert!(error.to_string().contains("controlled rekey required"));
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn bounded_ticket_reader_and_pending_store_permissions() {
        let directory = tempfile::tempdir().unwrap();
        let ticket = directory.path().join("oversize");
        std::fs::write(&ticket, vec![0; MAX_BOOTSTRAP_BYTES + 1]).unwrap();
        assert!(read_bounded_file(&ticket).is_err());
        prepare_directory(directory.path()).unwrap();
        prepare_delivery(directory.path(), &binding()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(directory.path())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            for file in ["managed-bootstrap.json", "managed-bootstrap-request.json"] {
                assert_eq!(
                    std::fs::metadata(directory.path().join(file))
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777,
                    0o600
                );
            }
            let path = directory.path().join("managed-bootstrap.json");
            let link = directory.path().join("recipient-link");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            assert!(read_private_file(&link).is_err());
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(read_private_file(&path).is_err());
        }
    }

    #[test]
    fn ordinary_identity_mutations_refuse_managed_or_pending_profiles_only() {
        let directory = tempfile::tempdir().unwrap();
        assert!(require_self_hosted_mutation(directory.path()).is_ok());
        let config = NodeConfig::new(
            "wss://example.test/api/v1/nodes/ws".into(),
            uuid::Uuid::new_v4().to_string(),
            "file".into(),
        );
        config.save(&directory.path().join("config.toml")).unwrap();
        assert!(require_self_hosted_mutation(directory.path()).is_ok());
        let binding = binding();
        managed_config(&binding)
            .save(&directory.path().join("config.toml"))
            .unwrap();
        assert!(require_self_hosted_mutation(directory.path()).is_err());
        std::fs::remove_file(directory.path().join("config.toml")).unwrap();
        prepare_delivery(directory.path(), &binding).unwrap();
        assert!(require_self_hosted_mutation(directory.path()).is_err());
    }

    #[tokio::test]
    async fn sealed_delivery_persists_encrypted_identity_and_restart_reuses_it() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let directory = tempfile::tempdir().unwrap();
        let config_dir = directory.path().join("node");
        let ticket_file = directory.path().join("ticket.json");
        let binding = binding();
        std::fs::write(
            &ticket_file,
            serde_json::to_vec(&serde_json::json!({"binding":binding,"token":"signed-fixture"}))
                .unwrap(),
        )
        .unwrap();
        let server = MockServer::start().await;
        let expected_binding = binding.clone();
        Mock::given(method("POST")).and(path("/bootstrap"))
            .respond_with(move |request: &wiremock::Request| {
                let request: Value = serde_json::from_slice(&request.body).unwrap();
                assert_eq!(request["binding"], serde_json::to_value(&expected_binding).unwrap());
                let public_key = request["recipient_public_key"].as_str().unwrap();
                let plaintext = serde_json::to_vec(&serde_json::json!({"auth_token":"nyx_nauth_fixture_only", "signing_secret":"a".repeat(64)})).unwrap();
                let sealed = sealed_bootstrap::seal(&plaintext, public_key, &expected_binding).unwrap();
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "binding":expected_binding, "sealed":sealed, "server_url":"wss://example.test/api/v1/nodes/ws", "writer_admitted":true,
                }))
            }).expect(1).mount(&server).await;
        let args = Start {
            ticket_file: Some(ticket_file),
            kubernetes: false,
            bootstrap_url: format!("{}/bootstrap", server.uri()),
            config: Some(config_dir.to_str().unwrap().into()),
        };
        assert_eq!(bootstrap_config(&args).await.unwrap(), config_dir);
        let config = NodeConfig::load(&config_dir.join("config.toml")).unwrap();
        validate_existing_config(&config, &binding).unwrap();
        let backend = SecretBackend::from_config(&config, &config_dir).unwrap();
        assert_eq!(
            backend.load_auth_token(&config).unwrap(),
            "nyx_nauth_fixture_only"
        );
        assert_eq!(
            backend.load_signing_secret(&config).unwrap().unwrap(),
            "a".repeat(64)
        );
        let stored = std::fs::read_to_string(config_dir.join("config.toml")).unwrap();
        assert!(!stored.contains("nyx_nauth_fixture_only") && !stored.contains(&"a".repeat(64)));
        assert!(!config_dir.join("managed-bootstrap.json").exists());
        assert!(!config_dir.join("managed-bootstrap-request.json").exists());
        assert_eq!(bootstrap_config(&args).await.unwrap(), config_dir);
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}
