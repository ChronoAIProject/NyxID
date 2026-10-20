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
    keychain::KeychainBackend,
    secret_backend::SecretBackend,
    secret_fields::SecretField,
    secret_operation::SecretOperation,
    storage::Directory,
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

/// One registration lease and directory observation through recipient creation,
/// network awaits and publication. An existing managed identity is read strictly
/// before any recipient key can be generated; only the Kubernetes fence advance
/// below authorizes its replacement.
struct DeliveryCustody {
    directory: Directory,
    config_path: PathBuf,
    _lease: std::fs::File,
    baseline: Option<NodeConfig>,
    operation: Option<SecretOperation>,
}

impl DeliveryCustody {
    fn acquire(path: &Path) -> Result<Self> {
        let directory = Directory::create_private(path)?;
        let config_path = path.join("config.toml");
        let baseline = match NodeConfig::load_in(&directory, &config_path) {
            Ok(config) => Some(config),
            Err(super::error::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                None
            }
            Err(error) => return Err(error.into()),
        };
        let operation = if let Some(config) = &baseline {
            let binding = config
                .managed_runtime
                .as_ref()
                .context("existing identity is not managed")?;
            validate_existing_config(config, binding)?;
            Some(SecretOperation::open_in(
                config,
                &config_path,
                directory.try_clone()?,
                KeychainBackend::new(&config.node.id),
            )?)
        } else {
            // A usable existing file key may be retained, but missing keys are
            // initialized only after a sealed, admitted delivery is verified.
            SecretBackend::preflight_in("file", &directory)?;
            None
        };
        // Invalid existing custody must refuse before even creating a
        // registration lease. Recheck the exact observation after acquisition.
        let lease = directory.registration_lock()?;
        {
            let _writer = directory.writer_lock()?;
            super::secret_transaction::check_observed(
                &directory,
                &config_path,
                baseline
                    .as_ref()
                    .and_then(|config| config.publication.snapshot()),
            )?;
            directory.make_private()?;
        }
        Ok(Self {
            directory,
            config_path,
            _lease: lease,
            baseline,
            operation,
        })
    }

    fn check(&self) -> Result<()> {
        if let (Some(operation), Some(config)) = (&self.operation, &self.baseline) {
            operation.health(config)?;
        }
        let _writer = self.directory.writer_lock()?;
        super::secret_transaction::check_observed(
            &self.directory,
            &self.config_path,
            self.baseline
                .as_ref()
                .and_then(|config| config.publication.snapshot()),
        )?;
        Ok(())
    }

    fn write<T: Serialize>(&self, name: &str, value: &T) -> Result<()> {
        self.check()?;
        let bytes = Zeroizing::new(serde_json::to_vec(value)?);
        ensure!(
            bytes.len() <= MAX_BOOTSTRAP_BYTES,
            "managed recovery file is too large"
        );
        let _writer = self.directory.writer_lock()?;
        super::secret_transaction::check_observed(
            &self.directory,
            &self.config_path,
            self.baseline
                .as_ref()
                .and_then(|config| config.publication.snapshot()),
        )?;
        // Existing metadata must be private/trusted too; never replace a
        // symlink, shared file or unexpected oversized artifact as a repair.
        let existing = read_optional(&self.directory, name)?;
        self.directory
            .publish(std::ffi::OsStr::new(name), &bytes, existing.is_some())?;
        Ok(())
    }
}

fn read_optional(directory: &Directory, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
    match directory.read(std::ffi::OsStr::new(name), MAX_BOOTSTRAP_BYTES as u64) {
        Ok(bytes) => Ok(Some(Zeroizing::new(bytes))),
        Err(super::error::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

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

fn kubernetes_recipient_in(
    setup: &KubernetesSetup,
    custody: &DeliveryCustody,
) -> Result<KubernetesRecipient> {
    custody.check()?;
    if let Some(bytes) = read_optional(&custody.directory, "managed-kubernetes-recipient.json")? {
        let recipient: KubernetesRecipient = serde_json::from_slice(&bytes)?;
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
    custody.write("managed-kubernetes-recipient.json", &recipient)?;
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
    let custody = DeliveryCustody::acquire(directory)?;
    if let Some(config) = &custody.baseline {
        ensure!(
            config.node.id == setup.node_id,
            "existing managed node does not match workload"
        );
    }
    let recipient_setup = setup.clone();
    let (mut custody, recipient) = tokio::task::spawn_blocking(move || {
        let recipient = kubernetes_recipient_in(&recipient_setup, &custody)?;
        Ok::<_, anyhow::Error>((custody, recipient))
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
            custody.check()?;
            if let Some(current) = &custody.baseline {
                let previous = current
                    .managed_runtime
                    .as_ref()
                    .context("existing identity is not managed")?;
                if previous == &binding {
                    validate_existing_config(current, &binding)?;
                    cleanup_delivery(&custody, &binding)?;
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
            if let Some(bytes) = read_optional(&custody.directory, "managed-bootstrap.json")? {
                let previous: PendingDelivery = serde_json::from_slice(&bytes)?;
                previous.binding.validate().map_err(anyhow::Error::msg)?;
                validate_recipient(&previous.private_key, &previous.public_key)?;
                ensure!(
                    previous.public_key == pending.public_key
                        && (previous.binding == binding
                            || (custody.baseline.is_some()
                                && previous.binding.node_id == binding.node_id
                                && previous.binding.sandbox_uid == binding.sandbox_uid
                                && previous.binding.pod_uid == binding.pod_uid
                                && (binding.runtime_generation
                                    > previous.binding.runtime_generation
                                    || (binding.runtime_generation
                                        == previous.binding.runtime_generation
                                        && binding.credential_epoch
                                            > previous.binding.credential_epoch)))),
                    "pending managed delivery binding changed"
                );
            }
            custody.write("managed-bootstrap.json", &pending)?;
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
            install_delivery(&mut custody, &pending, response)?;
            Ok(Some(directory.to_owned()))
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
    let custody = DeliveryCustody::acquire(&directory)?;
    ensure!(
        custody.baseline.is_none(),
        "managed credentials already activated; use controlled platform rekey"
    );
    let bytes = read_bounded_file(&args.ticket_file)?;
    let ticket: Ticket =
        serde_json::from_slice(&bytes).context("decode managed bootstrap ticket")?;
    ticket.binding.validate().map_err(anyhow::Error::msg)?;
    let pending =
        tokio::task::spawn_blocking(move || prepare_delivery_in(&custody, &ticket.binding))
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
    let custody = DeliveryCustody::acquire(&config_dir)?;
    let ticket_bytes = read_bounded_file(
        args.ticket_file
            .as_deref()
            .context("managed ticket file required")?,
    )
    .context("read managed bootstrap ticket")?;
    let ticket: Ticket =
        serde_json::from_slice(&ticket_bytes).context("decode managed bootstrap ticket")?;
    ticket.binding.validate().map_err(anyhow::Error::msg)?;
    if let Some(config) = &custody.baseline {
        validate_existing_config(config, &ticket.binding)?;
        cleanup_delivery(&custody, &ticket.binding)?;
        return Ok(config_dir);
    }
    let bootstrap_url = validate_endpoint(&args.bootstrap_url, false)?;
    let recipient_binding = ticket.binding.clone();
    let (mut custody, pending) = tokio::task::spawn_blocking(move || {
        let pending = prepare_delivery_in(&custody, &recipient_binding)?;
        Ok::<_, anyhow::Error>((custody, pending))
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
    install_delivery(&mut custody, &pending, response)?;
    Ok(config_dir)
}

fn install_delivery(
    custody: &mut DeliveryCustody,
    pending: &PendingDelivery,
    response: Response,
) -> Result<()> {
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
    let secrets: Secrets = serde_json::from_slice(&plaintext)
        .map_err(|_| anyhow::anyhow!("managed bootstrap secrets are invalid"))?;
    ensure!(
        !secrets.auth_token.is_empty() && !secrets.signing_secret.is_empty(),
        "managed bootstrap returned empty credentials"
    );
    // Validate the entire sealed capability contract before target key
    // initialization or any credential write.
    let machine = machine_config(&secrets.capabilities)?;

    custody.check()?;
    let mut config = custody.baseline.clone().unwrap_or_else(|| {
        NodeConfig::new(
            response.server_url.clone(),
            pending.binding.node_id.clone(),
            "file".into(),
        )
    });
    if let Some(previous) = &custody.baseline {
        let previous = previous
            .managed_runtime
            .as_ref()
            .context("existing identity is not managed")?;
        ensure!(
            previous.node_id == pending.binding.node_id
                && (pending.binding.runtime_generation > previous.runtime_generation
                    || (pending.binding.runtime_generation == previous.runtime_generation
                        && pending.binding.credential_epoch > previous.credential_epoch)),
            "managed rekey must advance the current workload fence"
        );
    }
    let operation = match custody.operation.take() {
        Some(operation) => operation,
        None => SecretOperation::registration_in(
            &config,
            &custody.config_path,
            custody.directory.try_clone()?,
            KeychainBackend::new(&pending.binding.node_id),
            custody._lease.try_clone()?,
        )?,
    };
    operation.prepare(&mut config)?;
    config.server.url = response.server_url;
    config.managed_runtime = Some(pending.binding.clone());
    config.machine = machine;
    let mut capabilities = secrets.capabilities;
    capabilities.sort();
    config.managed_capabilities = Some(capabilities);
    config.node.auth_token_encrypted =
        operation.store(SecretField::AuthToken, &secrets.auth_token)?;
    config.signing.shared_secret_encrypted =
        Some(operation.store(SecretField::SigningSecret, &secrets.signing_secret)?);
    operation.publish(&config, &custody.config_path)?;
    #[cfg(test)]
    if std::env::var_os("NYXID_TEST_MANAGED_EXIT_AFTER_INSTALL").is_some() {
        std::process::exit(0);
    }
    cleanup_delivery(custody, &pending.binding).context(
        "managed identity saved; pending delivery cleanup is incomplete; retain storage and retry managed startup",
    )
}

/// Retire only recovery metadata belonging to the installed, strictly readable
/// identity. Validate every artifact before removing any; a failed or uncertain
/// config publication never calls this path.
fn cleanup_delivery(
    custody: &DeliveryCustody,
    binding: &nyxid_machine::managed::Binding,
) -> Result<()> {
    let current = NodeConfig::load_in(&custody.directory, &custody.config_path)?;
    validate_existing_config(&current, binding)?;
    let _observation = SecretOperation::open_in(
        &current,
        &custody.config_path,
        custody.directory.try_clone()?,
        KeychainBackend::new(&current.node.id),
    )?;
    let _writer = custody.directory.writer_lock()?;
    super::secret_transaction::check_observed(
        &custody.directory,
        &custody.config_path,
        current.publication.snapshot(),
    )?;
    let mut owned = Vec::new();
    let mut public_key = None;
    if let Some(bytes) = read_optional(&custody.directory, "managed-bootstrap.json")? {
        let pending: PendingDelivery = serde_json::from_slice(&bytes).map_err(|_| {
            anyhow::anyhow!("managed pending delivery is invalid; preserve recovery files")
        })?;
        ensure!(
            &pending.binding == binding,
            "pending managed delivery belongs to another binding; preserve recovery files"
        );
        validate_recipient(&pending.private_key, &pending.public_key)?;
        public_key = Some(pending.public_key);
        owned.push("managed-bootstrap.json");
    }
    if let Some(bytes) = read_optional(&custody.directory, "managed-bootstrap-request.json")? {
        let request: Value = serde_json::from_slice(&bytes).map_err(|_| {
            anyhow::anyhow!("managed recipient request is invalid; preserve recovery files")
        })?;
        let key = request["recipient_public_key"]
            .as_str()
            .context("managed recipient request is invalid")?;
        let digest = sealed_bootstrap::recipient_key_digest(key)?;
        ensure!(
            request
                == serde_json::json!({
                    "binding": binding, "recipient_public_key": key, "recipient_key_digest": hex::encode(digest),
                })
                && public_key.as_deref().is_none_or(|expected| expected == key),
            "managed recipient request belongs to another delivery; preserve recovery files"
        );
        public_key = Some(key.to_owned());
        owned.push("managed-bootstrap-request.json");
    }
    if let Some(bytes) = read_optional(&custody.directory, "managed-kubernetes-recipient.json")? {
        let recipient: KubernetesRecipient = serde_json::from_slice(&bytes).map_err(|_| {
            anyhow::anyhow!("managed workload recipient is invalid; preserve recovery files")
        })?;
        ensure!(
            recipient.node_id == binding.node_id
                && recipient.pod_uid == binding.pod_uid
                && public_key
                    .as_deref()
                    .is_none_or(|key| key == recipient.public_key),
            "managed workload recipient belongs to another delivery; preserve recovery files"
        );
        validate_recipient(&recipient.private_key, &recipient.public_key)?;
        owned.push("managed-kubernetes-recipient.json");
    }
    custody.directory.sync()?;
    for name in owned {
        custody.directory.remove(std::ffi::OsStr::new(name))?;
    }
    Ok(())
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

fn prepare_delivery_in(
    custody: &DeliveryCustody,
    binding: &nyxid_machine::managed::Binding,
) -> Result<PendingDelivery> {
    custody.check()?;
    let pending = if let Some(bytes) = read_optional(&custody.directory, "managed-bootstrap.json")?
    {
        let pending: PendingDelivery =
            serde_json::from_slice(&bytes).context("decode pending managed delivery")?;
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
        custody.write("managed-bootstrap.json", &pending)?;
        pending
    };
    custody.write(
        "managed-bootstrap-request.json",
        &recipient_request(&pending)?,
    )?;
    Ok(pending)
}

#[cfg(test)]
fn atomic_private_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = Zeroizing::new(serde_json::to_vec(value)?);
    let directory = Directory::open(
        path.parent()
            .context("managed delivery path has no parent")?,
    )?;
    let _writer = directory.writer_lock()?;
    directory.publish(
        path.file_name()
            .context("managed delivery path has no name")?,
        &bytes,
        true,
    )?;
    Ok(())
}

#[cfg(test)]
fn prepare_directory(path: &Path) -> Result<()> {
    Directory::create_private(path)?.make_private()?;
    Ok(())
}

#[cfg(test)]
fn prepare_delivery(
    directory: &Path,
    binding: &nyxid_machine::managed::Binding,
) -> Result<PendingDelivery> {
    prepare_delivery_in(&DeliveryCustody::acquire(directory)?, binding)
}

#[cfg(test)]
fn kubernetes_recipient(setup: &KubernetesSetup, directory: &Path) -> Result<KubernetesRecipient> {
    kubernetes_recipient_in(setup, &DeliveryCustody::acquire(directory)?)
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
        install_fixture(&node, &previous);
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
        let error = bootstrap_kubernetes_with_setup(&setup, &node, Duration::from_secs(5))
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("managed bootstrap decryption failed"));
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

    /// A real encrypted source, including every optional credential class.
    /// Pure metadata fixtures above remain useful for capability validation.
    fn install_fixture(directory: &Path, binding: &nyxid_machine::managed::Binding) -> NodeConfig {
        use super::super::config::{CredentialConfig, PendingCryptoKeyConfig, SshKeyConfig};
        use super::super::secret_fields::{self, CredentialPart};
        prepare_directory(directory).unwrap();
        let mut config = managed_config(binding);
        config.signing.shared_secret_encrypted = Some(String::new());
        let mut oauth = CredentialConfig::new_header("Authorization".into(), None, None);
        oauth.oauth_managed = true;
        config.credentials.insert("oauth".into(), oauth);
        config.credentials.insert(
            "query".into(),
            CredentialConfig::new_query_param("key".into(), None, None),
        );
        config
            .credentials
            .insert("none".into(), CredentialConfig::new_no_auth(None));
        config.ssh_keys.push(SshKeyConfig {
            service_slug: "ssh".into(),
            principal: "alice".into(),
            private_key_pem_encrypted: None,
            passphrase_encrypted: None,
            target_host: "example.invalid".into(),
            target_port: 22,
            host_key_sha256: None,
            algorithms: None,
            created_at: "2026-10-20T00:00:00Z".into(),
        });
        config.pending_crypto_keys.insert(
            "pending".into(),
            PendingCryptoKeyConfig {
                version: "1".into(),
                service_slug: "pending-service".into(),
                injection_method: "header".into(),
                field_name: "Authorization".into(),
                target_url: None,
                expires_at: "2026-10-20T00:00:00Z".into(),
                public_key: "synthetic-public".into(),
                private_key_encrypted: None,
            },
        );
        let encryption = super::super::encryption::LocalEncryption::initialize(directory).unwrap();
        for (field, _) in secret_fields::fields(&config).unwrap() {
            let value = match &field {
                SecretField::SshPassphrase { .. } => None,
                SecretField::Credential {
                    part: CredentialPart::OAuthClientSecret,
                    ..
                } => Some(String::new()),
                _ => Some(format!(
                    "managed-sentinel:{}",
                    serde_json::to_string(&field).unwrap()
                )),
            };
            field
                .replace(
                    &mut config,
                    value.map(|value| encryption.encrypt(&value).unwrap()),
                )
                .unwrap();
        }
        let path = directory.join("config.toml");
        config.save(&path).unwrap();
        NodeConfig::load(&path).unwrap()
    }

    fn sealed_response(pending: &PendingDelivery, payload: Value) -> Response {
        Response {
            binding: pending.binding.clone(),
            sealed: sealed_bootstrap::seal(
                &serde_json::to_vec(&payload).unwrap(),
                &pending.public_key,
                &pending.binding,
            )
            .unwrap(),
            server_url: "wss://example.test/api/v1/nodes/ws".into(),
            writer_admitted: true,
        }
    }

    fn ticket_args(
        directory: &Path,
        binding: &nyxid_machine::managed::Binding,
        server: &str,
    ) -> Start {
        let ticket_file = directory.parent().unwrap().join("ticket.json");
        protected_token(
            &ticket_file,
            &serde_json::to_string(
                &serde_json::json!({"binding":binding,"token":"signed-fixture"}),
            )
            .unwrap(),
        );
        Start {
            ticket_file: Some(ticket_file),
            kubernetes: false,
            bootstrap_url: format!("{server}/bootstrap"),
            config: Some(directory.to_str().unwrap().into()),
        }
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
        let empty = tempfile::tempdir().unwrap();
        assert!(require_self_hosted_mutation(empty.path()).is_ok());

        let self_hosted = tempfile::tempdir().unwrap();
        let config = NodeConfig::new(
            "wss://example.test/api/v1/nodes/ws".into(),
            uuid::Uuid::new_v4().to_string(),
            "file".into(),
        );
        config
            .save(&self_hosted.path().join("config.toml"))
            .unwrap();
        assert!(require_self_hosted_mutation(self_hosted.path()).is_ok());

        let binding = binding();
        let managed = tempfile::tempdir().unwrap();
        managed_config(&binding)
            .save(&managed.path().join("config.toml"))
            .unwrap();
        assert!(require_self_hosted_mutation(managed.path()).is_err());

        let pending = tempfile::tempdir().unwrap();
        prepare_delivery(pending.path(), &binding).unwrap();
        assert!(require_self_hosted_mutation(pending.path()).is_err());
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
        let observed_directory = config_dir.clone();
        Mock::given(method("POST")).and(path("/bootstrap"))
            .respond_with(move |request: &wiremock::Request| {
                // The lease remains held across the actual provider await,
                // while local credential-key initialization has not occurred.
                assert!(Directory::open(&observed_directory).unwrap().registration_lock().is_err());
                assert!(!observed_directory.join(".keyfile").exists());
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

    #[tokio::test]
    async fn missing_managed_key_refuses_before_recipient_or_network_and_never_repairs() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("node");
        let binding = binding();
        install_fixture(&directory, &binding);
        let before = std::fs::read(directory.join("config.toml")).unwrap();
        std::fs::remove_file(directory.join(".keyfile")).unwrap();
        let server = wiremock::MockServer::start().await;
        let args = ticket_args(&directory, &binding, &server.uri());
        assert!(bootstrap_config(&args).await.is_err());
        let setup = KubernetesSetup {
            node_id: binding.node_id.clone(),
            pod_uid: binding.pod_uid.clone(),
            token_file: root.path().join("unread-token"),
            prepare_url: Url::parse(&format!("{}/prepare", server.uri())).unwrap(),
            bootstrap_url: Url::parse(&format!("{}/bootstrap", server.uri())).unwrap(),
        };
        assert!(
            bootstrap_kubernetes_with_setup(&setup, &directory, Duration::from_secs(5))
                .await
                .is_err()
        );
        assert!(server.received_requests().await.unwrap().is_empty());
        for name in [
            ".keyfile",
            ".registration.lock",
            "managed-bootstrap.json",
            "managed-bootstrap-request.json",
            "managed-kubernetes-recipient.json",
        ] {
            assert!(
                !directory.join(name).exists(),
                "unexpected custody effect: {name}"
            );
        }
        assert_eq!(
            std::fs::read(directory.join("config.toml")).unwrap(),
            before
        );
    }

    #[tokio::test]
    async fn existing_ordinary_identity_refuses_managed_preparation_before_any_recipient_effect() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("node");
        prepare_directory(&directory).unwrap();
        let binding = binding();
        NodeConfig::new(
            "wss://example.test/api/v1/nodes/ws".into(),
            binding.node_id.clone(),
            "file".into(),
        )
        .save(&directory.join("config.toml"))
        .unwrap();
        let server = wiremock::MockServer::start().await;
        let args = ticket_args(&directory, &binding, &server.uri());
        assert!(
            bootstrap_config(&args)
                .await
                .unwrap_err()
                .to_string()
                .contains("not managed")
        );
        assert!(
            prepare(Prepare {
                ticket_file: args.ticket_file.unwrap(),
                config: args.config
            })
            .await
            .is_err()
        );
        assert!(server.received_requests().await.unwrap().is_empty());
        for name in [
            ".keyfile",
            ".registration.lock",
            "managed-bootstrap.json",
            "managed-bootstrap-request.json",
        ] {
            assert!(!directory.join(name).exists());
        }
    }

    #[tokio::test]
    async fn managed_provider_await_cannot_retarget_recipient_publication_or_cleanup() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("node");
        let retained = root.path().join("retained");
        let binding = binding();
        let server = MockServer::start().await;
        let original = directory.clone();
        let moved = retained.clone();
        let expected = binding.clone();
        Mock::given(method("POST")).and(path("/bootstrap")).respond_with(move |request: &wiremock::Request| {
            let request: Value = serde_json::from_slice(&request.body).unwrap();
            assert!(Directory::open(&original).unwrap().registration_lock().is_err());
            std::fs::rename(&original, &moved).unwrap();
            prepare_directory(&original).unwrap();
            protected_token(&original.join("managed-bootstrap.json"), "replacement recovery marker");
            protected_token(&original.join(".keyfile"), "replacement key marker");
            let payload = serde_json::to_vec(&serde_json::json!({"auth_token":"nyx_nauth_original_directory","signing_secret":"c".repeat(64)})).unwrap();
            let sealed = sealed_bootstrap::seal(&payload, request["recipient_public_key"].as_str().unwrap(), &expected).unwrap();
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"binding":expected,"sealed":sealed,"server_url":"wss://example.test/api/v1/nodes/ws","writer_admitted":true}))
        }).expect(1).mount(&server).await;
        bootstrap_config(&ticket_args(&directory, &binding, &server.uri()))
            .await
            .unwrap();
        assert!(!directory.join("config.toml").exists());
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 2);
        assert_eq!(
            std::fs::read(directory.join(".keyfile")).unwrap(),
            b"replacement key marker"
        );
        assert_eq!(
            std::fs::read(directory.join("managed-bootstrap.json")).unwrap(),
            b"replacement recovery marker"
        );
        let installed = NodeConfig::load(&retained.join("config.toml")).unwrap();
        let backend = SecretBackend::from_config(&installed, &retained).unwrap();
        assert_eq!(
            backend.load_auth_token(&installed).unwrap(),
            "nyx_nauth_original_directory"
        );
        assert!(!retained.join("managed-bootstrap.json").exists());
        assert!(!retained.join("managed-bootstrap-request.json").exists());
        Directory::open(&retained)
            .unwrap()
            .registration_lock()
            .unwrap();
    }

    #[tokio::test]
    async fn config_appearing_during_managed_await_preserves_pending_and_never_initializes_key() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("node");
        let binding = binding();
        let server = MockServer::start().await;
        let target = directory.clone();
        let expected = binding.clone();
        Mock::given(method("POST")).and(path("/bootstrap")).respond_with(move |request: &wiremock::Request| {
            let request: Value = serde_json::from_slice(&request.body).unwrap();
            NodeConfig::new("wss://other.example.test/api/v1/nodes/ws".into(), "competing-node".into(), "file".into())
                .save(&target.join("config.toml")).unwrap();
            let payload = serde_json::to_vec(&serde_json::json!({"auth_token":"nyx_nauth_stale","signing_secret":"d".repeat(64)})).unwrap();
            let sealed = sealed_bootstrap::seal(&payload, request["recipient_public_key"].as_str().unwrap(), &expected).unwrap();
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"binding":expected,"sealed":sealed,"server_url":"wss://example.test/api/v1/nodes/ws","writer_admitted":true}))
        }).expect(1).mount(&server).await;
        assert!(
            bootstrap_config(&ticket_args(&directory, &binding, &server.uri()))
                .await
                .is_err()
        );
        assert!(!directory.join(".keyfile").exists());
        assert_eq!(
            NodeConfig::load(&directory.join("config.toml"))
                .unwrap()
                .node
                .id,
            "competing-node"
        );
        assert!(directory.join("managed-bootstrap.json").exists());
        assert!(directory.join("managed-bootstrap-request.json").exists());
    }

    #[test]
    fn invalid_sealed_capabilities_never_initialize_target_custody() {
        let root = tempfile::tempdir().unwrap();
        let mut custody = DeliveryCustody::acquire(root.path()).unwrap();
        let pending = prepare_delivery_in(&custody, &binding()).unwrap();
        let response = sealed_response(
            &pending,
            serde_json::json!({"auth_token":"nyx_nauth_invalid_capability","signing_secret":"e".repeat(64),"capabilities":["unrecognized"]}),
        );
        assert!(install_delivery(&mut custody, &pending, response).is_err());
        assert!(!root.path().join(".keyfile").exists());
        assert!(!root.path().join("config.toml").exists());
        assert!(root.path().join("managed-bootstrap.json").exists());
    }

    #[tokio::test]
    async fn installed_identity_with_corrupt_cleanup_metadata_resumes_without_provider_replay() {
        let root = tempfile::tempdir().unwrap();
        let binding = binding();
        let mut custody = DeliveryCustody::acquire(&root.path().join("node")).unwrap();
        let pending = prepare_delivery_in(&custody, &binding).unwrap();
        let request = custody
            .directory
            .read(
                std::ffi::OsStr::new("managed-bootstrap-request.json"),
                MAX_BOOTSTRAP_BYTES as u64,
            )
            .unwrap();
        custody
            .write(
                "managed-bootstrap-request.json",
                &serde_json::json!({"foreign":true}),
            )
            .unwrap();
        let response = sealed_response(
            &pending,
            serde_json::json!({"auth_token":"nyx_nauth_retained","signing_secret":"f".repeat(64)}),
        );
        let error = install_delivery(&mut custody, &pending, response).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("identity saved; pending delivery cleanup is incomplete")
        );
        let directory = root.path().join("node");
        let installed = std::fs::read(directory.join("config.toml")).unwrap();
        // Validation precedes every retirement, including when later metadata
        // is corrupt. Correcting only that metadata makes restart idempotent.
        assert!(directory.join("managed-bootstrap.json").exists());
        assert!(directory.join("managed-bootstrap-request.json").exists());
        custody
            .directory
            .publish(
                std::ffi::OsStr::new("managed-bootstrap-request.json"),
                &request,
                true,
            )
            .unwrap();
        drop(custody);
        let server = wiremock::MockServer::start().await;
        bootstrap_config(&ticket_args(&directory, &binding, &server.uri()))
            .await
            .unwrap();
        assert!(server.received_requests().await.unwrap().is_empty());
        assert_eq!(
            std::fs::read(directory.join("config.toml")).unwrap(),
            installed
        );
        assert!(!directory.join("managed-bootstrap.json").exists());
        assert!(!directory.join("managed-bootstrap-request.json").exists());
    }

    #[tokio::test]
    async fn admitted_kubernetes_rekey_preserves_every_existing_credential_class() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("node");
        let previous = binding();
        let before = install_fixture(&directory, &previous);
        let path_config = directory.join("config.toml");
        let source =
            SecretOperation::open(&before, &path_config, KeychainBackend::new(&before.node.id))
                .unwrap();
        let mut next = previous.clone();
        next.credential_epoch += 1;
        next.delivery_id = uuid::Uuid::new_v4().to_string();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/prepare"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"binding":next})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let expected = next.clone();
        Mock::given(method("POST")).and(path("/bootstrap")).respond_with(move |request: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let payload = serde_json::to_vec(&serde_json::json!({"auth_token":"nyx_nauth_rekeyed","signing_secret":"a".repeat(64)})).unwrap();
            let sealed = sealed_bootstrap::seal(&payload, body["recipient_public_key"].as_str().unwrap(), &expected).unwrap();
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"binding":expected,"sealed":sealed,"server_url":"wss://example.test/api/v1/nodes/ws","writer_admitted":true}))
        }).expect(1).mount(&server).await;
        let token_file = root.path().join("projected-token");
        protected_token(&token_file, "synthetic-projected-token");
        let setup = KubernetesSetup {
            node_id: previous.node_id,
            pod_uid: previous.pod_uid,
            token_file,
            prepare_url: Url::parse(&format!("{}/prepare", server.uri())).unwrap(),
            bootstrap_url: Url::parse(&format!("{}/bootstrap", server.uri())).unwrap(),
        };
        bootstrap_kubernetes_with_setup(&setup, &directory, Duration::from_secs(5))
            .await
            .unwrap();
        let after = NodeConfig::load(&path_config).unwrap();
        validate_existing_config(&after, &next).unwrap();
        let target =
            SecretOperation::open(&after, &path_config, KeychainBackend::new(&after.node.id))
                .unwrap();
        let old_encryption =
            super::super::encryption::LocalEncryption::open_existing(&directory).unwrap();
        for (field, _) in super::super::secret_fields::fields(&before).unwrap() {
            if matches!(field, SecretField::AuthToken | SecretField::SigningSecret) {
                continue;
            }
            // Source loads require their exact generation, so compare with
            // the old encrypted projection using its unchanged selected key.
            let old = field
                .encoded(&before)
                .unwrap()
                .map(|value| old_encryption.decrypt(value).unwrap());
            assert_eq!(
                target
                    .load(&after, &field)
                    .unwrap()
                    .as_deref()
                    .map(|value| value.as_str()),
                old.as_deref()
            );
        }
        assert!(source.health(&before).is_err()); // Original CAS is stale.
        assert_eq!(
            target
                .load(&after, &SecretField::AuthToken)
                .unwrap()
                .unwrap()
                .as_str(),
            "nyx_nauth_rekeyed"
        );
        assert_eq!(
            target
                .load(&after, &SecretField::SigningSecret)
                .unwrap()
                .unwrap()
                .as_str(),
            "a".repeat(64)
        );
        assert_eq!(after.credentials["none"].injection_method, "none");
        assert!(after.ssh_keys[0].passphrase_encrypted.is_none());
        assert!(!directory.join("managed-bootstrap.json").exists());
        assert!(!directory.join("managed-kubernetes-recipient.json").exists());
    }

    #[test]
    fn managed_install_crash_child() {
        let Some(path) = std::env::var_os("NYXID_TEST_MANAGED_EXIT_AFTER_INSTALL") else {
            return;
        };
        let mut custody = DeliveryCustody::acquire(Path::new(&path)).unwrap();
        let pending = prepare_delivery_in(&custody, &binding()).unwrap();
        let response = sealed_response(
            &pending,
            serde_json::json!({"auth_token":"nyx_nauth_crash_fixture","signing_secret":"b".repeat(64)}),
        );
        install_delivery(&mut custody, &pending, response).unwrap();
        panic!("post-install crash hook was not reached");
    }

    #[tokio::test]
    async fn process_exit_after_managed_install_resumes_exact_bytes_without_provider_contact() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("node");
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "node::managed_runtime::tests::managed_install_crash_child",
                "--nocapture",
            ])
            .env("NYXID_TEST_MANAGED_EXIT_AFTER_INSTALL", &directory)
            .output()
            .unwrap();
        assert!(child.status.success());
        assert!(!String::from_utf8_lossy(&child.stdout).contains("nyx_nauth_crash_fixture"));
        assert!(!String::from_utf8_lossy(&child.stderr).contains("nyx_nauth_crash_fixture"));
        let installed = std::fs::read(directory.join("config.toml")).unwrap();
        let config = NodeConfig::load(&directory.join("config.toml")).unwrap();
        assert!(directory.join("managed-bootstrap.json").exists());
        assert!(directory.join("managed-bootstrap-request.json").exists());
        let server = wiremock::MockServer::start().await;
        bootstrap_config(&ticket_args(
            &directory,
            config.managed_runtime.as_ref().unwrap(),
            &server.uri(),
        ))
        .await
        .unwrap();
        assert!(server.received_requests().await.unwrap().is_empty());
        assert_eq!(
            std::fs::read(directory.join("config.toml")).unwrap(),
            installed
        );
        assert!(!directory.join("managed-bootstrap.json").exists());
        assert!(!directory.join("managed-bootstrap-request.json").exists());
    }

    #[test]
    fn managed_rekey_refuses_same_or_backward_fence_without_mutating_source_or_recovery() {
        for advance in [0, -1] {
            let root = tempfile::tempdir().unwrap();
            let mut previous = binding();
            previous.credential_epoch = 2;
            install_fixture(root.path(), &previous);
            let before = std::fs::read(root.path().join("config.toml")).unwrap();
            let mut custody = DeliveryCustody::acquire(root.path()).unwrap();
            let mut next = previous;
            if advance < 0 {
                next.credential_epoch -= 1;
            }
            next.delivery_id = uuid::Uuid::new_v4().to_string();
            let pending = prepare_delivery_in(&custody, &next).unwrap();
            let recovery = std::fs::read(root.path().join("managed-bootstrap.json")).unwrap();
            let response = sealed_response(
                &pending,
                serde_json::json!({"auth_token":"nyx_nauth_denied_rekey","signing_secret":"c".repeat(64)}),
            );
            assert!(
                install_delivery(&mut custody, &pending, response)
                    .unwrap_err()
                    .to_string()
                    .contains("must advance")
            );
            assert_eq!(
                std::fs::read(root.path().join("config.toml")).unwrap(),
                before
            );
            assert_eq!(
                std::fs::read(root.path().join("managed-bootstrap.json")).unwrap(),
                recovery
            );
        }
    }

    #[test]
    fn cleanup_refuses_foreign_pending_binding_and_preserves_all_artifacts() {
        let root = tempfile::tempdir().unwrap();
        let binding = binding();
        install_fixture(root.path(), &binding);
        let custody = DeliveryCustody::acquire(root.path()).unwrap();
        let mut other = binding.clone();
        other.delivery_id = uuid::Uuid::new_v4().to_string();
        prepare_delivery_in(&custody, &other).unwrap();
        let before = std::fs::read(root.path().join("managed-bootstrap.json")).unwrap();
        let request = std::fs::read(root.path().join("managed-bootstrap-request.json")).unwrap();
        assert!(
            cleanup_delivery(&custody, &binding)
                .unwrap_err()
                .to_string()
                .contains("another binding")
        );
        assert_eq!(
            std::fs::read(root.path().join("managed-bootstrap.json")).unwrap(),
            before
        );
        assert_eq!(
            std::fs::read(root.path().join("managed-bootstrap-request.json")).unwrap(),
            request
        );
    }

    #[tokio::test]
    async fn kubernetes_rekey_cannot_overwrite_foreign_pending_delivery_even_with_same_recipient() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("node");
        let previous = binding();
        install_fixture(&directory, &previous);
        let before = std::fs::read(directory.join("config.toml")).unwrap();
        let mut next = previous.clone();
        next.credential_epoch += 1;
        next.delivery_id = uuid::Uuid::new_v4().to_string();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/prepare"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"binding":next})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let token_file = root.path().join("token");
        protected_token(&token_file, "fixture-projected-token");
        let setup = KubernetesSetup {
            node_id: previous.node_id.clone(),
            pod_uid: previous.pod_uid.clone(),
            token_file,
            prepare_url: Url::parse(&format!("{}/prepare", server.uri())).unwrap(),
            bootstrap_url: Url::parse(&format!("{}/bootstrap", server.uri())).unwrap(),
        };
        let custody = DeliveryCustody::acquire(&directory).unwrap();
        let recipient = kubernetes_recipient_in(&setup, &custody).unwrap();
        let mut foreign = previous;
        foreign.sandbox_uid = uuid::Uuid::new_v4().to_string();
        custody
            .write(
                "managed-bootstrap.json",
                &PendingDelivery {
                    binding: foreign,
                    private_key: recipient.private_key,
                    public_key: recipient.public_key,
                },
            )
            .unwrap();
        let retained = std::fs::read(directory.join("managed-bootstrap.json")).unwrap();
        drop(custody);
        assert!(
            bootstrap_kubernetes_with_setup(&setup, &directory, Duration::from_secs(5))
                .await
                .unwrap_err()
                .to_string()
                .contains("bootstrap refused")
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        assert_eq!(
            std::fs::read(directory.join("config.toml")).unwrap(),
            before
        );
        assert_eq!(
            std::fs::read(directory.join("managed-bootstrap.json")).unwrap(),
            retained
        );
    }
}
