use std::io::Write;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::api::ApiClient;
use crate::cli::{NodeCredentialAdminCommands, OutputFormat, PendingCredentialInjectionMethod};

const RCI_PUBKEY_TIMEOUT: Duration = Duration::from_secs(30);
const RCI_TERMINAL_TIMEOUT: Duration = Duration::from_secs(60);
const RCI_POLL_DELAY: Duration = Duration::from_millis(250);

pub async fn run(command: NodeCredentialAdminCommands) -> Result<()> {
    match command {
        NodeCredentialAdminCommands::Push {
            node,
            slug,
            injection_method,
            field_name,
            target_url,
            label,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let node_id = crate::commands::node::resolve_node_id(&mut api, &node).await?;
            let body = serde_json::json!({
                "service_slug": slug,
                "injection_method": injection_method.wire_value(),
                "field_name": field_name,
                "target_url": target_url,
                "label": label,
            });
            let pending: Value = api
                .post(&format!("/nodes/{node_id}/credentials/push"), &body)
                .await?;

            match auth.output {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&pending)?),
                OutputFormat::Table => {
                    let pending_id = pending["id"].as_str().unwrap_or("-");
                    let slug = pending["service_slug"].as_str().unwrap_or("-");
                    let method = pending["injection_method"].as_str().unwrap_or("-");
                    let field = pending["field_name"].as_str().unwrap_or("-");
                    eprintln!("Pending credential created: {pending_id}");
                    eprintln!();
                    eprintln!("Relay this setup metadata to the VM operator:");
                    eprintln!("  slug: {slug}");
                    eprintln!("  injection method: {method}");
                    eprintln!("  field name: {field}");
                    eprintln!();
                    eprintln!(
                        "Remote injection can complete this pending credential without sending the secret value to NyxID."
                    );
                    eprintln!(
                        "Legacy node-side path: SSH to the node-agent machine and run `nyxid node credentials pending`, then `nyxid node credentials accept {slug}`."
                    );
                    eprintln!(
                        "`nyxid node credentials` is node-side only; it is not available on the user-side CLI."
                    );
                    eprintln!(
                        "push sends only credential metadata; the secret is not sent to NyxID."
                    );
                    eprintln!();
                    for line in RciCliHintLines::push_continuation_lines(
                        api.base_url_root(),
                        &node_id,
                        pending_id,
                    ) {
                        eprintln!("{line}");
                    }
                }
            }
            Ok(())
        }
        NodeCredentialAdminCommands::Inject {
            node,
            pending,
            slug,
            injection_method,
            field_name,
            target_url,
            label,
            org,
            secret_env,
            browser,
            verify_fingerprint,
            yes,
            auth,
        } => {
            if pending.is_some()
                && (slug.is_some()
                    || injection_method.is_some()
                    || field_name.is_some()
                    || target_url.is_some()
                    || label.is_some())
            {
                bail!(
                    "--pending reuses existing metadata; do not pass --slug, --injection-method, --field-name, --target-url, or --label"
                );
            }
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let node_id = resolve_node_id_for_optional_org(&mut api, &node, org.as_deref()).await?;
            let metadata = match (slug, injection_method, field_name, target_url, label) {
                (None, None, None, None, None) => None,
                (slug, injection_method, field_name, target_url, label) => Some(InjectMetadata {
                    slug: slug.ok_or_else(|| {
                        anyhow::anyhow!("--slug is required unless --pending is used")
                    })?,
                    injection_method: injection_method.ok_or_else(|| {
                        anyhow::anyhow!("--injection-method is required unless --pending is used")
                    })?,
                    field_name: field_name.ok_or_else(|| {
                        anyhow::anyhow!("--field-name is required unless --pending is used")
                    })?,
                    target_url,
                    label,
                }),
            };
            let secret_source = if browser {
                RciSecretSource::BrowserOnly
            } else if let Some(env_name) = secret_env {
                RciSecretSource::Env(env_name)
            } else {
                RciSecretSource::Prompt
            };
            let fingerprint_policy = if let Some(expected) = verify_fingerprint {
                RciFingerprintPolicy::Expect(expected)
            } else if yes {
                RciFingerprintPolicy::SkipWithYes
            } else {
                RciFingerprintPolicy::ConfirmPrompt
            };
            let mode = if browser {
                RciInjectMode::Browser
            } else {
                RciInjectMode::Terminal
            };
            let mut session = RciInjectSession::new(
                &mut api,
                node_id,
                pending,
                metadata,
                secret_source,
                fingerprint_policy,
                mode,
            );
            let outcome = session.run().await?;
            match auth.output {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&outcome)?),
                OutputFormat::Table => print_inject_outcome(&outcome),
            }
            Ok(())
        }
        NodeCredentialAdminCommands::List { node, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let node_id = crate::commands::node::resolve_node_id(&mut api, &node).await?;
            let response: Value = api
                .get(&format!("/nodes/{node_id}/credentials/pending"))
                .await?;

            match auth.output {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&response)?),
                OutputFormat::Table => {
                    let pending = response
                        .get("pending_credentials")
                        .and_then(|value| value.as_array())
                        .cloned()
                        .unwrap_or_default();
                    if pending.is_empty() {
                        eprintln!("No pending credentials for this node.");
                        return Ok(());
                    }

                    let mut table = Table::new();
                    table.load_preset(UTF8_FULL_CONDENSED);
                    table.set_header(["ID", "Slug", "Method", "Field", "Age", "Expires"]);
                    for item in pending {
                        let id = item["id"].as_str().unwrap_or("-");
                        let slug = item["service_slug"].as_str().unwrap_or("-");
                        let method = item["injection_method"].as_str().unwrap_or("-");
                        let field = item["field_name"].as_str().unwrap_or("-");
                        let created_at = item["created_at"].as_str().unwrap_or("-");
                        let expires_at = item["expires_at"].as_str().unwrap_or("-");
                        table.add_row([
                            id.to_string(),
                            slug.to_string(),
                            method.to_string(),
                            field.to_string(),
                            format_age(created_at),
                            expires_at.to_string(),
                        ]);
                    }
                    eprintln!("{table}");
                }
            }
            Ok(())
        }
        NodeCredentialAdminCommands::Cancel {
            node,
            pending_id,
            yes,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let node_id = crate::commands::node::resolve_node_id(&mut api, &node).await?;

            if !yes {
                eprintln!("Cancel pending credential: {pending_id}");
                eprintln!("Node: {node}");
                eprint!("Proceed? [y/N] ");
                std::io::stderr().flush()?;
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;
                if !answer.trim().eq_ignore_ascii_case("y") {
                    eprintln!("Cancelled.");
                    return Ok(());
                }
            }

            api.delete_empty(&format!(
                "/nodes/{node_id}/credentials/pending/{pending_id}"
            ))
            .await?;

            match auth.output {
                OutputFormat::Json => println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "id": pending_id,
                        "canceled": true,
                    }))?
                ),
                OutputFormat::Table => eprintln!("Pending credential canceled."),
            }
            Ok(())
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct InjectMetadata {
    slug: String,
    injection_method: PendingCredentialInjectionMethod,
    field_name: String,
    target_url: Option<String>,
    label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RciSecretSource {
    Prompt,
    Env(String),
    BrowserOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RciFingerprintPolicy {
    ConfirmPrompt,
    Expect(String),
    SkipWithYes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RciInjectMode {
    Terminal,
    Browser,
}

#[derive(Debug, Deserialize, Clone)]
struct PendingCredentialInfoCli {
    id: String,
    #[serde(default)]
    node_id: Option<String>,
    service_slug: String,
    injection_method: String,
    field_name: String,
    #[serde(default)]
    target_url: Option<String>,
    expires_at: String,
    #[serde(default)]
    consumed_at: Option<String>,
    #[serde(default)]
    declined_at: Option<String>,
    #[serde(default)]
    remote_state: Option<String>,
    #[serde(default = "default_true")]
    is_active: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
struct PendingCredentialListResponseCli {
    pending_credentials: Vec<PendingCredentialInfoCli>,
}

#[derive(Debug, Deserialize)]
struct PendingCredentialPubkeyResponseCli {
    pending_id: String,
    node_id: String,
    service_slug: String,
    version: String,
    node_pubkey: String,
    #[serde(default)]
    remote_state: Option<String>,
    #[serde(default)]
    integrity_verification_opt_out: bool,
}

#[derive(Debug, Deserialize)]
struct PendingCredentialCiphertextResponseCli {
    delivery_status: String,
    remote_state: String,
    #[serde(default)]
    error_code: Option<u32>,
}

#[derive(Debug, Serialize)]
struct PendingCredentialCiphertextPost {
    version: String,
    admin_pubkey: String,
    nonce: String,
    ciphertext: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    integrity_verification: Option<Value>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct RciInjectOutcome {
    pending_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    browser_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    remote_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    delivery_status: Option<String>,
}

struct RciInjectSession<'a> {
    api: &'a mut ApiClient,
    node_id: String,
    pending_id: Option<String>,
    metadata: Option<InjectMetadata>,
    secret_source: RciSecretSource,
    fingerprint_policy: RciFingerprintPolicy,
    mode: RciInjectMode,
    pubkey_timeout: Duration,
    terminal_timeout: Duration,
    poll_delay: Duration,
    prompt_secret_override: Option<Zeroizing<String>>,
}

impl<'a> RciInjectSession<'a> {
    fn new(
        api: &'a mut ApiClient,
        node_id: String,
        pending_id: Option<String>,
        metadata: Option<InjectMetadata>,
        secret_source: RciSecretSource,
        fingerprint_policy: RciFingerprintPolicy,
        mode: RciInjectMode,
    ) -> Self {
        Self {
            api,
            node_id,
            pending_id,
            metadata,
            secret_source,
            fingerprint_policy,
            mode,
            pubkey_timeout: RCI_PUBKEY_TIMEOUT,
            terminal_timeout: RCI_TERMINAL_TIMEOUT,
            poll_delay: RCI_POLL_DELAY,
            prompt_secret_override: None,
        }
    }

    #[cfg(test)]
    fn with_prompt_secret_override(mut self, secret: Zeroizing<String>) -> Self {
        self.prompt_secret_override = Some(secret);
        self
    }

    #[cfg(test)]
    fn with_timeouts(mut self, pubkey_timeout: Duration, terminal_timeout: Duration) -> Self {
        self.pubkey_timeout = pubkey_timeout;
        self.terminal_timeout = terminal_timeout;
        self
    }

    async fn run(&mut self) -> Result<RciInjectOutcome> {
        self.validate()?;
        let pending = self.init_or_create_pending().await?;
        match self.mode {
            RciInjectMode::Browser => self.run_browser(pending).await,
            RciInjectMode::Terminal => self.run_terminal(pending).await,
        }
    }

    fn validate(&self) -> Result<()> {
        if self.pending_id.is_some() && self.metadata.is_some() {
            bail!(
                "--pending reuses existing metadata; do not pass --slug, --injection-method, --field-name, --target-url, or --label"
            );
        }
        if self.pending_id.is_none() && self.metadata.is_none() {
            bail!(
                "--slug, --injection-method, and --field-name are required unless --pending is used"
            );
        }
        if matches!(self.secret_source, RciSecretSource::Env(_))
            && matches!(self.fingerprint_policy, RciFingerprintPolicy::ConfirmPrompt)
        {
            bail!(
                "--secret-env requires --verify-fingerprint or --yes so automation does not block on a prompt"
            );
        }
        Ok(())
    }

    async fn init_or_create_pending(&mut self) -> Result<PendingCredentialInfoCli> {
        if let Some(pending_id) = self.pending_id.as_deref() {
            return self
                .api
                .post(
                    &format!(
                        "/nodes/{}/credentials/pending/{pending_id}/remote-crypto",
                        self.node_id
                    ),
                    &serde_json::json!({}),
                )
                .await
                .with_context(|| {
                    format!(
                        "Failed to initialize pending credential {pending_id} for remote crypto"
                    )
                });
        }

        let metadata = self
            .metadata
            .as_ref()
            .context("missing injection metadata")?;
        let body = serde_json::json!({
            "service_slug": metadata.slug,
            "injection_method": metadata.injection_method.wire_value(),
            "field_name": metadata.field_name,
            "target_url": metadata.target_url,
            "label": metadata.label,
            "remote_crypto": true,
        });
        self.api
            .post(&format!("/nodes/{}/credentials/push", self.node_id), &body)
            .await
            .context("Failed to create remote-crypto pending credential")
    }

    async fn run_browser(&mut self, pending: PendingCredentialInfoCli) -> Result<RciInjectOutcome> {
        let url = pending_accept_url(self.api.base_url_root(), &self.node_id, &pending.id);
        eprintln!("Open this credential injection page:");
        eprintln!("  {url}");
        eprintln!(
            "Security: standalone accept-page release-integrity is detection-only and does not prevent T1 node pubkey substitution."
        );
        eprintln!(
            "Security: org policy remote_credential_integrity_verification_opt_out skips or relaxes that integrity verification."
        );
        if std::env::var_os("NYXID_WIZARD_NO_OPEN").is_none() {
            if let Err(error) = crate::browser::open_browser(&url) {
                eprintln!("Could not open browser automatically: {error}");
            }
        } else {
            eprintln!("  (NYXID_WIZARD_NO_OPEN set -- not opening a browser)");
        }

        let completed = self.poll_terminal_state(&pending.id).await?;
        Ok(RciInjectOutcome {
            pending_id: pending.id,
            browser_url: Some(url),
            remote_state: terminal_state(&completed).or(completed.remote_state),
            delivery_status: None,
        })
    }

    async fn run_terminal(
        &mut self,
        pending: PendingCredentialInfoCli,
    ) -> Result<RciInjectOutcome> {
        let pubkey = self.poll_pubkey(&pending.id).await?;
        validate_pubkey_response_matches_pending(&pending, &pubkey)?;
        self.verify_fingerprint(&pubkey)?;
        let secret = self.read_secret()?;
        let body = encrypt_pending_secret_for_node(&pending, &pubkey, &secret)?;
        let response: PendingCredentialCiphertextResponseCli = self
            .api
            .post(
                &format!(
                    "/nodes/{}/credentials/pending/{}/ciphertext",
                    self.node_id, pending.id
                ),
                &body,
            )
            .await
            .context("Failed to submit encrypted pending credential")?;

        if is_terminal_remote_state(&response.remote_state) {
            if let Some(code) = response.error_code {
                print_delivery_error_code(code);
            }
            return Ok(RciInjectOutcome {
                pending_id: pending.id,
                browser_url: None,
                remote_state: Some(response.remote_state),
                delivery_status: Some(response.delivery_status),
            });
        }

        let completed = self.poll_terminal_state(&pending.id).await?;
        if let Some(code) = response.error_code {
            print_delivery_error_code(code);
        }
        Ok(RciInjectOutcome {
            pending_id: pending.id,
            browser_url: None,
            remote_state: terminal_state(&completed).or(completed.remote_state),
            delivery_status: Some(response.delivery_status),
        })
    }

    async fn poll_pubkey(
        &mut self,
        pending_id: &str,
    ) -> Result<PendingCredentialPubkeyResponseCli> {
        let start = tokio::time::Instant::now();
        loop {
            match self
                .api
                .get::<PendingCredentialPubkeyResponseCli>(&format!(
                    "/nodes/{}/credentials/pending/{pending_id}",
                    self.node_id
                ))
                .await
            {
                Ok(response) => return Ok(response),
                Err(error) if is_pubkey_awaiting_error(&error) => {
                    if start.elapsed() >= self.pubkey_timeout {
                        bail!(
                            "Timed out waiting for node pubkey for pending credential {pending_id}. Ensure the node agent is online and supports remote credential injection."
                        );
                    }
                    tokio::time::sleep(self.poll_delay).await;
                }
                Err(error) => return Err(error),
            }
        }
    }

    async fn poll_terminal_state(&mut self, pending_id: &str) -> Result<PendingCredentialInfoCli> {
        let start = tokio::time::Instant::now();
        loop {
            let response: PendingCredentialListResponseCli = self
                .api
                .get(&format!(
                    "/nodes/{}/credentials/pending?include_history=true",
                    self.node_id
                ))
                .await
                .context("Failed to poll pending credential status")?;
            if let Some(pending) = response
                .pending_credentials
                .into_iter()
                .find(|item| item.id == pending_id)
                .filter(|pending| terminal_state(pending).is_some())
            {
                return Ok(pending);
            }

            if start.elapsed() >= self.terminal_timeout {
                bail!(
                    "Timed out waiting for pending credential {pending_id} to reach a terminal state"
                );
            }
            tokio::time::sleep(self.poll_delay).await;
        }
    }

    fn verify_fingerprint(&self, pubkey: &PendingCredentialPubkeyResponseCli) -> Result<()> {
        let fingerprint = nyxid_crypto::rci_pubkey_fingerprint_b64u(&pubkey.node_pubkey)
            .context("Invalid node pubkey received from backend")?;
        match &self.fingerprint_policy {
            RciFingerprintPolicy::Expect(expected) => {
                let expected = normalize_fingerprint_input(expected)?;
                if fingerprint != expected {
                    bail!(
                        "Node pubkey fingerprint mismatch: expected {expected}, got {fingerprint}"
                    );
                }
                eprintln!("Node pubkey fingerprint matched --verify-fingerprint.");
            }
            RciFingerprintPolicy::SkipWithYes => {
                eprintln!("Node pubkey fingerprint: {fingerprint}");
                eprintln!(
                    "Skipping out-of-band fingerprint confirmation because --yes was passed; T1 data-substitution is not checked."
                );
            }
            RciFingerprintPolicy::ConfirmPrompt => {
                eprintln!("Node pubkey fingerprint: {fingerprint}");
                eprintln!("Compare this with the node-agent console fingerprint out-of-band.");
                eprint!("Confirm this fingerprint matches the node console? [y/N] ");
                std::io::stderr().flush()?;
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;
                if !answer.trim().eq_ignore_ascii_case("y") {
                    bail!("Node pubkey fingerprint was not confirmed");
                }
            }
        }
        Ok(())
    }

    fn read_secret(&mut self) -> Result<Zeroizing<String>> {
        match &self.secret_source {
            RciSecretSource::Prompt => {
                if let Some(secret) = self.prompt_secret_override.take() {
                    return ensure_non_empty_secret(secret);
                }
                let value = rpassword::prompt_password("Enter secret value: ")
                    .context("Failed to read secret value")?;
                ensure_non_empty_secret(Zeroizing::new(value))
            }
            RciSecretSource::Env(name) => {
                let value = std::env::var(name)
                    .with_context(|| format!("Environment variable '{name}' is not set"))?;
                ensure_non_empty_secret(Zeroizing::new(value))
            }
            RciSecretSource::BrowserOnly => {
                bail!("internal error: browser mode must not read a terminal secret")
            }
        }
    }
}

fn encrypt_pending_secret_for_node(
    pending: &PendingCredentialInfoCli,
    pubkey: &PendingCredentialPubkeyResponseCli,
    secret: &Zeroizing<String>,
) -> Result<PendingCredentialCiphertextPost> {
    let node_pubkey = nyxid_crypto::decode_b64u_array::<32>("node_pubkey", &pubkey.node_pubkey)
        .context("Invalid node pubkey received from backend")?;
    let node_id = pending
        .node_id
        .as_deref()
        .unwrap_or(&pubkey.node_id)
        .to_string();
    let metadata = crate::node::credentials::crypto::PendingCredentialCryptoMetadata {
        pending_id: pending.id.clone(),
        node_id,
        service_slug: pending.service_slug.clone(),
        injection_method: pending.injection_method.clone(),
        field_name: pending.field_name.clone(),
        target_url: pending.target_url.clone(),
        expires_at: pending.expires_at.clone(),
        version: pubkey.version.clone(),
    };
    let plaintext = Zeroizing::new(secret.as_bytes().to_vec());
    let envelope = nyxid_crypto::encrypt(plaintext.as_slice(), node_pubkey, &metadata.context())
        .context("Failed to encrypt pending credential")?;
    Ok(PendingCredentialCiphertextPost {
        version: envelope.version,
        admin_pubkey: nyxid_crypto::encode_b64u(&envelope.admin_pubkey),
        nonce: nyxid_crypto::encode_b64u(&envelope.nonce),
        ciphertext: nyxid_crypto::encode_b64u(&envelope.ciphertext),
        integrity_verification: if pubkey.integrity_verification_opt_out {
            Some(serde_json::json!({
                "mode": "org_policy_opt_out",
                "fingerprint_sha384_hex": null,
                "verified_at": null,
                "manifest_url_configured": false,
            }))
        } else {
            None
        },
    })
}

fn validate_pubkey_response_matches_pending(
    pending: &PendingCredentialInfoCli,
    pubkey: &PendingCredentialPubkeyResponseCli,
) -> Result<()> {
    if pubkey.pending_id != pending.id {
        bail!(
            "Backend returned node pubkey for pending credential {}, expected {}",
            pubkey.pending_id,
            pending.id
        );
    }
    if pubkey.service_slug != pending.service_slug {
        bail!(
            "Backend returned node pubkey for service {}, expected {}",
            pubkey.service_slug,
            pending.service_slug
        );
    }
    if let Some(state) = pubkey.remote_state.as_deref()
        && is_terminal_remote_state(state)
    {
        bail!(
            "Pending credential {} is already in terminal remote state {state}",
            pending.id
        );
    }
    Ok(())
}

fn ensure_non_empty_secret(secret: Zeroizing<String>) -> Result<Zeroizing<String>> {
    if secret.is_empty() {
        bail!("Secret value must not be empty");
    }
    Ok(secret)
}

fn is_pubkey_awaiting_error(error: &anyhow::Error) -> bool {
    let text = error.to_string();
    text.contains("8009") || text.contains("pending_credential_pubkey_awaiting")
}

fn normalize_fingerprint_input(input: &str) -> Result<String> {
    let trimmed = input.trim();
    if trimmed.len() != 32
        || !trimmed
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("Node pubkey fingerprint must be bare 32-character lowercase hex");
    }
    Ok(trimmed.to_string())
}

fn terminal_state(pending: &PendingCredentialInfoCli) -> Option<String> {
    if let Some(state) = pending.remote_state.as_deref()
        && is_terminal_remote_state(state)
    {
        return Some(state.to_string());
    }
    if pending.consumed_at.is_some() {
        return Some("consumed".to_string());
    }
    if pending.declined_at.is_some() {
        return Some("declined".to_string());
    }
    if !pending.is_active {
        return Some("expired".to_string());
    }
    None
}

fn is_terminal_remote_state(state: &str) -> bool {
    matches!(
        state,
        "consumed" | "decrypt_failed" | "expired" | "declined" | "partial_decrypted"
    )
}

fn pending_accept_url(base_url_root: &str, node_id: &str, pending_id: &str) -> String {
    format!(
        "{}/nodes/{}/credentials/pending/{}/accept",
        base_url_root.trim_end_matches('/'),
        node_id,
        pending_id
    )
}

pub(crate) struct RciCliHintLines;

impl RciCliHintLines {
    pub(crate) fn rci_service_hint_lines(node_id: Option<&str>, slug: &str) -> Vec<String> {
        let node_arg = node_id
            .filter(|value| !value.is_empty())
            .unwrap_or("<node-id>");
        vec![
            "Remote credential injection (secret is not sent to NyxID):".to_string(),
            format!(
                "Create metadata: nyxid node-credential push {node_arg} --slug {slug} --injection-method <header|query-param|path-prefix> --field-name <name> [--target-url <url>] [--label <label>]"
            ),
            format!(
                "Complete pending: nyxid node-credential inject {node_arg} --pending <pending-id> [--browser | --secret-env VAR] [--verify-fingerprint <32 lowercase hex> | --yes] [--org <ID|SLUG|NAME>]"
            ),
            format!(
                "One-step create+inject: nyxid node-credential inject {node_arg} --slug {slug} --injection-method <header|query-param|path-prefix> --field-name <name> [--target-url <url>] [--label <label>] [--browser | --secret-env VAR] [--verify-fingerprint <32 lowercase hex> | --yes] [--org <ID|SLUG|NAME>]"
            ),
            "Security: CLI injection avoids browser-JS code substitution, but NyxID still relays the node pubkey.".to_string(),
            "Security: compare the node-agent console fingerprint out-of-band with --verify-fingerprint <32 lowercase hex>; --yes skips that check.".to_string(),
            "Security: standalone accept-page release-integrity is detection-only and does not prevent T1 pubkey substitution.".to_string(),
            "Security: org policy remote_credential_integrity_verification_opt_out skips or relaxes that integrity verification.".to_string(),
        ]
    }

    pub(crate) fn push_continuation_lines(
        base_url_root: &str,
        node_id: &str,
        pending_id: &str,
    ) -> Vec<String> {
        vec![
            "Continuation options:".to_string(),
            format!(
                "  Standalone accept page: {}",
                pending_accept_url(base_url_root, node_id, pending_id)
            ),
            format!(
                "  CLI: nyxid node-credential inject {node_id} --pending {pending_id} [--browser | --secret-env VAR] [--verify-fingerprint <32 lowercase hex> | --yes] [--org <ID|SLUG|NAME>]"
            ),
            format!(
                "  CLI browser: nyxid node-credential inject {node_id} --pending {pending_id} --browser [--verify-fingerprint <32 lowercase hex> | --yes] [--org <ID|SLUG|NAME>]"
            ),
            "  push sends only credential metadata; the secret is not sent to NyxID.".to_string(),
            "  CLI injection avoids browser-JS code substitution, but NyxID still relays the node pubkey.".to_string(),
            "  Compare the node-agent console fingerprint out-of-band with --verify-fingerprint <32 lowercase hex>; --yes skips that check.".to_string(),
            "  Standalone accept-page release-integrity is detection-only and does not prevent T1 pubkey substitution.".to_string(),
            "  Org policy remote_credential_integrity_verification_opt_out skips or relaxes that integrity verification.".to_string(),
        ]
    }

    pub(crate) fn rci_delivery_error_hint(code: u32) -> Option<&'static str> {
        match code {
            8006 => Some("decrypt failed/AAD verify failed"),
            8007 => Some("version unsupported (protocol drift)"),
            8008 => Some("ciphertext too large"),
            8009 => Some("pubkey awaiting"),
            8010 => Some("node offline; ciphertext queued"),
            8011 => Some("queue full"),
            _ => None,
        }
    }
}

fn print_delivery_error_code(code: u32) {
    if let Some(hint) = RciCliHintLines::rci_delivery_error_hint(code) {
        eprintln!("Backend delivery error code: {code} ({hint})");
    } else {
        eprintln!("Backend delivery error code: {code}");
    }
}

#[derive(Debug, Deserialize)]
struct NodeListResponseCli {
    nodes: Vec<Value>,
}

async fn resolve_node_id_for_optional_org(
    api: &mut ApiClient,
    id_or_name: &str,
    org: Option<&str>,
) -> Result<String> {
    let Some(org_input) = org else {
        return crate::commands::node::resolve_node_id(api, id_or_name).await;
    };
    if Uuid::parse_str(id_or_name).is_ok() {
        return Ok(id_or_name.to_string());
    }

    let org_id = crate::org_resolver::resolve_org_id(api, org_input).await?;
    let response: NodeListResponseCli = api.get("/nodes").await?;
    let mut matches = response
        .nodes
        .iter()
        .filter(|node| node["name"].as_str() == Some(id_or_name))
        .filter(|node| node["owner"]["id"].as_str() == Some(org_id.as_str()));
    if let Some(node) = matches.next() {
        if matches.next().is_some() {
            bail!(
                "Multiple nodes named '{id_or_name}' are visible for org {org_id}; pass the node UUID"
            );
        }
        return node["id"]
            .as_str()
            .or(node["_id"].as_str())
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!("Node '{id_or_name}' found but has no ID"));
    }

    bail!("Node '{id_or_name}' not found for org {org_id}")
}

fn print_inject_outcome(outcome: &RciInjectOutcome) {
    eprintln!("Pending credential injection submitted.");
    eprintln!("Pending ID: {}", outcome.pending_id);
    if let Some(state) = outcome.remote_state.as_deref() {
        eprintln!("Remote state: {state}");
    }
    if let Some(status) = outcome.delivery_status.as_deref() {
        eprintln!("Delivery: {status}");
    }
    if let Some(url) = outcome.browser_url.as_deref() {
        eprintln!("Browser URL: {url}");
    }
}

fn format_age(created_at: &str) -> String {
    let Ok(created) = chrono::DateTime::parse_from_rfc3339(created_at) else {
        return "-".to_string();
    };
    let age = chrono::Utc::now().signed_duration_since(created.with_timezone(&chrono::Utc));
    let seconds = age.num_seconds().max(0);
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3_600 {
        format!("{}m", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h", seconds / 3_600)
    } else {
        format!("{}d", seconds / 86_400)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        InjectMetadata, PendingCredentialInfoCli, PendingCredentialPubkeyResponseCli,
        RciCliHintLines, RciFingerprintPolicy, RciInjectMode, RciInjectSession, RciSecretSource,
        encrypt_pending_secret_for_node, format_age, normalize_fingerprint_input,
        pending_accept_url, validate_pubkey_response_matches_pending,
    };
    use crate::api::ApiClient;
    use crate::cli::{NodeCredentialAdminCommands, OutputFormat, PendingCredentialInjectionMethod};
    use crate::test_support::{mock_auth, mock_auth_with_output};
    use serde_json::json;
    use std::time::Duration;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use zeroize::Zeroizing;

    #[test]
    fn format_age_handles_invalid_timestamp() {
        assert_eq!(format_age("not-a-date"), "-");
    }

    #[test]
    fn format_age_reports_seconds_minutes_hours_and_days() {
        let now = chrono::Utc::now();
        assert_eq!(
            format_age(&(now - chrono::Duration::seconds(42)).to_rfc3339()),
            "42s"
        );
        assert_eq!(
            format_age(&(now - chrono::Duration::minutes(17)).to_rfc3339()),
            "17m"
        );
        assert_eq!(
            format_age(&(now - chrono::Duration::hours(5)).to_rfc3339()),
            "5h"
        );
        assert_eq!(
            format_age(&(now - chrono::Duration::days(3)).to_rfc3339()),
            "3d"
        );
    }

    #[test]
    fn format_age_clamps_future_timestamps_to_zero_seconds() {
        let future = chrono::Utc::now() + chrono::Duration::minutes(5);
        assert_eq!(format_age(&future.to_rfc3339()), "0s");
    }

    #[tokio::test]
    async fn push_resolves_node_name_and_posts_pending_credential_metadata() {
        let server = MockServer::start().await;
        let node_id = "dbf51e02-633d-4293-a896-ec0fb383f30b";

        Mock::given(method("GET"))
            .and(path("/api/v1/nodes"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "nodes": [
                    {"id": node_id, "name": "edge-node"}
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path(format!("/api/v1/nodes/{node_id}/credentials/push")))
            .and(body_json(json!({
                "service_slug": "openai",
                "injection_method": "header",
                "field_name": "Authorization",
                "target_url": "https://api.openai.com/v1",
                "label": "OpenAI production"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "pending-1",
                "service_slug": "openai",
                "injection_method": "header",
                "field_name": "Authorization"
            })))
            .expect(1)
            .mount(&server)
            .await;

        super::run(NodeCredentialAdminCommands::Push {
            node: "edge-node".to_string(),
            slug: "openai".to_string(),
            injection_method: PendingCredentialInjectionMethod::Header,
            field_name: "Authorization".to_string(),
            target_url: Some("https://api.openai.com/v1".to_string()),
            label: Some("OpenAI production".to_string()),
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("push should succeed");
    }

    #[tokio::test]
    async fn list_fetches_pending_credentials_for_resolved_node() {
        let server = MockServer::start().await;
        let node_id = "dbf51e02-633d-4293-a896-ec0fb383f30b";

        Mock::given(method("GET"))
            .and(path("/api/v1/nodes"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "nodes": [
                    {"id": node_id, "name": "edge-node"}
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path(format!("/api/v1/nodes/{node_id}/credentials/pending")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "pending_credentials": [
                    {
                        "id": "pending-1",
                        "service_slug": "openai",
                        "injection_method": "header",
                        "field_name": "Authorization",
                        "created_at": "2026-01-01T00:00:00Z",
                        "expires_at": "2026-01-01T01:00:00Z"
                    }
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;

        super::run(NodeCredentialAdminCommands::List {
            node: "edge-node".to_string(),
            auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
        })
        .await
        .expect("list should succeed");
    }

    #[tokio::test]
    async fn list_handles_empty_pending_credentials_response() {
        let server = MockServer::start().await;
        let node_id = "dbf51e02-633d-4293-a896-ec0fb383f30b";

        Mock::given(method("GET"))
            .and(path(format!("/api/v1/nodes/{node_id}/credentials/pending")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "pending_credentials": []
            })))
            .expect(1)
            .mount(&server)
            .await;

        super::run(NodeCredentialAdminCommands::List {
            node: node_id.to_string(),
            auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
        })
        .await
        .expect("empty list should succeed");
    }

    #[tokio::test]
    async fn cancel_with_yes_deletes_pending_credential_without_prompt() {
        let server = MockServer::start().await;
        let node_id = "dbf51e02-633d-4293-a896-ec0fb383f30b";

        Mock::given(method("DELETE"))
            .and(path(format!(
                "/api/v1/nodes/{node_id}/credentials/pending/pending-1"
            )))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        super::run(NodeCredentialAdminCommands::Cancel {
            node: node_id.to_string(),
            pending_id: "pending-1".to_string(),
            yes: true,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("cancel should succeed");
    }

    fn node_id() -> &'static str {
        "dbf51e02-633d-4293-a896-ec0fb383f30b"
    }

    fn inject_metadata() -> InjectMetadata {
        InjectMetadata {
            slug: "openai".to_string(),
            injection_method: PendingCredentialInjectionMethod::Header,
            field_name: "Authorization".to_string(),
            target_url: Some("https://api.openai.com/v1".to_string()),
            label: Some("OpenAI production".to_string()),
        }
    }

    fn pending_info_json(pending_id: &str) -> serde_json::Value {
        json!({
            "id": pending_id,
            "node_id": node_id(),
            "service_slug": "openai",
            "injection_method": "header",
            "field_name": "Authorization",
            "target_url": "https://api.openai.com/v1",
            "expires_at": "2099-01-01T00:00:00Z",
            "remote_state": "pubkey_awaiting",
            "is_active": true
        })
    }

    fn consumed_pending_list(pending_id: &str) -> serde_json::Value {
        json!({
            "pending_credentials": [{
                "id": pending_id,
                "node_id": node_id(),
                "service_slug": "openai",
                "injection_method": "header",
                "field_name": "Authorization",
                "target_url": "https://api.openai.com/v1",
                "expires_at": "2099-01-01T00:00:00Z",
                "remote_state": "consumed",
                "consumed_at": "2026-06-05T00:00:00Z",
                "is_active": false
            }]
        })
    }

    fn pubkey_response_json(
        pending_id: &str,
        keypair: &nyxid_crypto::NodeKeypair,
    ) -> serde_json::Value {
        json!({
            "pending_id": pending_id,
            "node_id": node_id(),
            "service_slug": "openai",
            "version": nyxid_crypto::VERSION_V1,
            "node_pubkey": keypair.public_key_b64u(),
            "remote_state": "pubkey_posted",
            "integrity_verification_opt_out": true
        })
    }

    fn test_api(server: &MockServer) -> ApiClient {
        ApiClient::new(&server.uri(), "test-access-token".to_string()).expect("api client")
    }

    async fn mount_new_inject_flow(
        server: &MockServer,
        pending_id: &str,
        keypair: &nyxid_crypto::NodeKeypair,
    ) {
        Mock::given(method("POST"))
            .and(path(format!(
                "/api/v1/nodes/{}/credentials/push",
                node_id()
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(pending_info_json(pending_id)))
            .expect(1)
            .mount(server)
            .await;

        Mock::given(method("GET"))
            .and(path(format!(
                "/api/v1/nodes/{}/credentials/pending/{pending_id}",
                node_id()
            )))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(pubkey_response_json(pending_id, keypair)),
            )
            .expect(1)
            .mount(server)
            .await;

        Mock::given(method("POST"))
            .and(path(format!(
                "/api/v1/nodes/{}/credentials/pending/{pending_id}/ciphertext",
                node_id()
            )))
            .respond_with(ResponseTemplate::new(202).set_body_json(json!({
                "delivery_status": "sent",
                "remote_state": "consumed"
            })))
            .expect(1)
            .mount(server)
            .await;
    }

    async fn mount_pending_inject_flow(
        server: &MockServer,
        pending_id: &str,
        keypair: &nyxid_crypto::NodeKeypair,
    ) {
        Mock::given(method("POST"))
            .and(path(format!(
                "/api/v1/nodes/{}/credentials/pending/{pending_id}/remote-crypto",
                node_id()
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(pending_info_json(pending_id)))
            .expect(1)
            .mount(server)
            .await;

        Mock::given(method("GET"))
            .and(path(format!(
                "/api/v1/nodes/{}/credentials/pending/{pending_id}",
                node_id()
            )))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(pubkey_response_json(pending_id, keypair)),
            )
            .expect(1)
            .mount(server)
            .await;

        Mock::given(method("POST"))
            .and(path(format!(
                "/api/v1/nodes/{}/credentials/pending/{pending_id}/ciphertext",
                node_id()
            )))
            .respond_with(ResponseTemplate::new(202).set_body_json(json!({
                "delivery_status": "sent",
                "remote_state": "consumed"
            })))
            .expect(1)
            .mount(server)
            .await;
    }

    async fn assert_no_request_body_contains(server: &MockServer, needle: &str) {
        let received = server
            .received_requests()
            .await
            .expect("request recording enabled");
        for request in received {
            let body = String::from_utf8_lossy(&request.body);
            assert!(
                !body.contains(needle),
                "request {} {} leaked forbidden body text",
                request.method,
                request.url.path()
            );
        }
    }

    async fn ciphertext_request_body(server: &MockServer) -> serde_json::Value {
        let received = server
            .received_requests()
            .await
            .expect("request recording enabled");
        let request = received
            .iter()
            .find(|request| request.url.path().ends_with("/ciphertext"))
            .expect("ciphertext request");
        request.body_json().expect("ciphertext JSON")
    }

    #[test]
    fn push_fallback_prints_all_three_paths() {
        let lines = RciCliHintLines::push_continuation_lines(
            "https://nyxid.example",
            "node-1",
            "pending-1",
        );
        let rendered = lines.join("\n");

        assert!(rendered.contains("Continuation options:"));
        assert!(
            rendered.contains(
                "https://nyxid.example/nodes/node-1/credentials/pending/pending-1/accept"
            )
        );
        assert!(rendered.contains("nyxid node-credential inject node-1 --pending pending-1"));
        assert!(
            rendered.contains("nyxid node-credential inject node-1 --pending pending-1 --browser")
        );
        assert!(rendered.contains("--secret-env VAR"));
        assert!(rendered.contains("--verify-fingerprint <32 lowercase hex>"));
        assert!(rendered.contains("push sends only credential metadata"));
        assert!(rendered.contains("the secret is not sent to NyxID"));
    }

    #[test]
    fn rci_delivery_error_hints_cover_reserved_codes() {
        let cases = [
            (8006, ["decrypt", "AAD verify failed"]),
            (8007, ["version unsupported", "protocol drift"]),
            (8008, ["ciphertext too large", "ciphertext too large"]),
            (8009, ["pubkey awaiting", "pubkey awaiting"]),
            (8010, ["node offline", "ciphertext queued"]),
            (8011, ["queue full", "queue full"]),
        ];

        let mut rendered = String::new();
        for (code, expected) in cases {
            let hint =
                RciCliHintLines::rci_delivery_error_hint(code).expect("reserved code has hint");
            assert!(hint.contains(expected[0]), "hint {code}: {hint}");
            assert!(hint.contains(expected[1]), "hint {code}: {hint}");
            rendered.push_str(hint);
            rendered.push('\n');
        }

        assert!(RciCliHintLines::rci_delivery_error_hint(8005).is_none());
        assert!(RciCliHintLines::rci_delivery_error_hint(8012).is_none());
        assert!(!rendered.contains("pending_credential_integrity_failed"));
    }

    #[test]
    fn normalize_fingerprint_accepts_only_bare_lowercase_hex32() {
        let valid = "0123456789abcdef0123456789abcdef";
        assert_eq!(normalize_fingerprint_input(valid).unwrap(), valid);
        assert_eq!(
            normalize_fingerprint_input(" 0123456789abcdef0123456789abcdef\n").unwrap(),
            valid
        );

        for invalid in [
            format!("sha256:{valid}"),
            format!("SHA256:{valid}"),
            valid.to_ascii_uppercase(),
            format!("{valid}{valid}"),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa==".to_string(),
        ] {
            assert!(
                normalize_fingerprint_input(&invalid).is_err(),
                "accepted invalid fingerprint input: {invalid}"
            );
        }
    }

    #[tokio::test]
    async fn inject_pending_rejects_metadata_overrides() {
        let err = super::run(NodeCredentialAdminCommands::Inject {
            node: node_id().to_string(),
            pending: Some("pending-1".to_string()),
            slug: Some("openai".to_string()),
            injection_method: None,
            field_name: None,
            target_url: None,
            label: None,
            org: None,
            secret_env: None,
            browser: false,
            verify_fingerprint: None,
            yes: true,
            auth: mock_auth("http://127.0.0.1:9"),
        })
        .await
        .expect_err("metadata overrides should be rejected before API use");

        assert!(
            err.to_string()
                .contains("--pending reuses existing metadata")
        );
    }

    #[tokio::test]
    async fn inject_secret_env_requires_verify_or_yes_without_prompt() {
        let server = MockServer::start().await;
        let mut api = test_api(&server);
        let mut session = RciInjectSession::new(
            &mut api,
            node_id().to_string(),
            None,
            Some(inject_metadata()),
            RciSecretSource::Env("RCI_SECRET_NOT_READ".to_string()),
            RciFingerprintPolicy::ConfirmPrompt,
            RciInjectMode::Terminal,
        );

        let err = session
            .run()
            .await
            .expect_err("--secret-env should require non-interactive fingerprint policy");

        assert!(
            err.to_string()
                .contains("--secret-env requires --verify-fingerprint or --yes")
        );
        assert!(
            server
                .received_requests()
                .await
                .expect("requests")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn inject_verify_fingerprint_mismatch_aborts_before_secret_read() {
        let server = MockServer::start().await;
        let keypair = nyxid_crypto::generate_node_keypair();
        Mock::given(method("POST"))
            .and(path(format!(
                "/api/v1/nodes/{}/credentials/push",
                node_id()
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(pending_info_json("pending-1")))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!(
                "/api/v1/nodes/{}/credentials/pending/pending-1",
                node_id()
            )))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(pubkey_response_json("pending-1", &keypair)),
            )
            .expect(1)
            .mount(&server)
            .await;
        let mut api = test_api(&server);
        let mut session = RciInjectSession::new(
            &mut api,
            node_id().to_string(),
            None,
            Some(inject_metadata()),
            RciSecretSource::Env("RCI_SECRET_NOT_SET".to_string()),
            RciFingerprintPolicy::Expect("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()),
            RciInjectMode::Terminal,
        );

        let err = session
            .run()
            .await
            .expect_err("fingerprint mismatch should abort");

        assert!(err.to_string().contains("fingerprint mismatch"));
        assert!(!err.to_string().contains("RCI_SECRET_NOT_SET"));
        let received = server
            .received_requests()
            .await
            .expect("request recording enabled");
        assert!(
            !received
                .iter()
                .any(|request| request.url.path().ends_with("/ciphertext"))
        );
    }

    #[tokio::test]
    async fn inject_interactive_full_flow() {
        let server = MockServer::start().await;
        let keypair = nyxid_crypto::generate_node_keypair();
        let fingerprint = nyxid_crypto::rci_pubkey_fingerprint(keypair.public_key());
        mount_new_inject_flow(&server, "pending-1", &keypair).await;
        let mut api = test_api(&server);
        let mut session = RciInjectSession::new(
            &mut api,
            node_id().to_string(),
            None,
            Some(inject_metadata()),
            RciSecretSource::Prompt,
            RciFingerprintPolicy::Expect(fingerprint),
            RciInjectMode::Terminal,
        )
        .with_prompt_secret_override(Zeroizing::new("Bearer sk-rci".to_string()));

        let outcome = session.run().await.expect("inject should complete");

        assert_eq!(outcome.pending_id, "pending-1");
        assert_eq!(outcome.remote_state.as_deref(), Some("consumed"));
        let body = ciphertext_request_body(&server).await;
        assert_eq!(body["version"], "v1");
        assert_eq!(body["integrity_verification"]["mode"], "org_policy_opt_out");
        assert!(!body.to_string().contains("sk-rci"));
        assert_no_request_body_contains(&server, "sk-rci").await;
        let received = server.received_requests().await.expect("requests");
        let push: serde_json::Value = received
            .iter()
            .find(|request| request.url.path().ends_with("/credentials/push"))
            .expect("push request")
            .body_json()
            .expect("push json");
        assert_eq!(push["remote_crypto"], true);
    }

    #[tokio::test(flavor = "current_thread")]
    #[allow(clippy::await_holding_lock)] // env_lock serializes env mutation across single-thread test; held across await by design
    async fn inject_secret_env_reads_from_env() {
        let _guard = crate::test_support::env_lock().lock().unwrap();
        // SAFETY: env mutation is serialized by env_lock and this test runs on one thread.
        unsafe {
            std::env::set_var("NYXID_RCI_TEST_SECRET", "Bearer sk-env-rci");
        }
        let server = MockServer::start().await;
        let keypair = nyxid_crypto::generate_node_keypair();
        mount_new_inject_flow(&server, "pending-1", &keypair).await;
        let mut api = test_api(&server);
        let mut session = RciInjectSession::new(
            &mut api,
            node_id().to_string(),
            None,
            Some(inject_metadata()),
            RciSecretSource::Env("NYXID_RCI_TEST_SECRET".to_string()),
            RciFingerprintPolicy::SkipWithYes,
            RciInjectMode::Terminal,
        );

        session.run().await.expect("env inject should complete");

        assert_no_request_body_contains(&server, "sk-env-rci").await;
        // SAFETY: env mutation is serialized by env_lock and this test runs on one thread.
        unsafe {
            std::env::remove_var("NYXID_RCI_TEST_SECRET");
        }
    }

    #[tokio::test]
    async fn inject_pubkey_timeout_errors_cleanly() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!(
                "/api/v1/nodes/{}/credentials/pending/pending-1/remote-crypto",
                node_id()
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(pending_info_json("pending-1")))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!(
                "/api/v1/nodes/{}/credentials/pending/pending-1",
                node_id()
            )))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": "pending_credential_pubkey_awaiting",
                "code": 8009
            })))
            .expect(1)
            .mount(&server)
            .await;
        let mut api = test_api(&server);
        let mut session = RciInjectSession::new(
            &mut api,
            node_id().to_string(),
            Some("pending-1".to_string()),
            None,
            RciSecretSource::Prompt,
            RciFingerprintPolicy::SkipWithYes,
            RciInjectMode::Terminal,
        )
        .with_timeouts(Duration::ZERO, Duration::ZERO);

        let err = session
            .run()
            .await
            .expect_err("pubkey timeout should fail cleanly");

        assert!(
            err.to_string()
                .contains("Timed out waiting for node pubkey")
        );
        assert!(!err.to_string().contains("secret"));
    }

    #[tokio::test(flavor = "current_thread")]
    #[allow(clippy::await_holding_lock)] // env_lock serializes env mutation across single-thread test; held across await by design
    async fn inject_org_flag_resolves_correctly() {
        let _guard = crate::test_support::env_lock().lock().unwrap();
        // SAFETY: env mutation is serialized by env_lock and this test runs on one thread.
        unsafe {
            std::env::set_var("NYXID_WIZARD_NO_OPEN", "1");
        }
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/orgs/acme"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "org-1",
                "slug": "acme",
                "display_name": "Acme"
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/nodes"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "nodes": [
                    {"id": "other-node", "name": "edge-node", "owner": {"id": "user-1", "kind": "user"}},
                    {"id": node_id(), "name": "edge-node", "owner": {"id": "org-1", "kind": "org"}}
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(format!(
                "/api/v1/nodes/{}/credentials/pending/pending-1/remote-crypto",
                node_id()
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(pending_info_json("pending-1")))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!(
                "/api/v1/nodes/{}/credentials/pending",
                node_id()
            )))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(consumed_pending_list("pending-1")),
            )
            .expect(1)
            .mount(&server)
            .await;

        super::run(NodeCredentialAdminCommands::Inject {
            node: "edge-node".to_string(),
            pending: Some("pending-1".to_string()),
            slug: None,
            injection_method: None,
            field_name: None,
            target_url: None,
            label: None,
            org: Some("acme".to_string()),
            secret_env: None,
            browser: true,
            verify_fingerprint: None,
            yes: true,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("org-scoped browser inject should succeed");

        let received = server.received_requests().await.expect("requests");
        assert!(received.iter().any(|request| {
            request.url.path()
                == format!(
                    "/api/v1/nodes/{}/credentials/pending/pending-1/remote-crypto",
                    node_id()
                )
        }));
        // SAFETY: env mutation is serialized by env_lock and this test runs on one thread.
        unsafe {
            std::env::remove_var("NYXID_WIZARD_NO_OPEN");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    #[allow(clippy::await_holding_lock)] // env_lock serializes env mutation across single-thread test; held across await by design
    async fn inject_browser_wizard_opens_url_and_polls() {
        let _guard = crate::test_support::env_lock().lock().unwrap();
        // SAFETY: env mutation is serialized by env_lock and this test runs on one thread.
        unsafe {
            std::env::set_var("NYXID_WIZARD_NO_OPEN", "1");
        }
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!(
                "/api/v1/nodes/{}/credentials/push",
                node_id()
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(pending_info_json("pending-1")))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!(
                "/api/v1/nodes/{}/credentials/pending",
                node_id()
            )))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(consumed_pending_list("pending-1")),
            )
            .expect(1)
            .mount(&server)
            .await;
        let mut api = test_api(&server);
        let mut session = RciInjectSession::new(
            &mut api,
            node_id().to_string(),
            None,
            Some(inject_metadata()),
            RciSecretSource::BrowserOnly,
            RciFingerprintPolicy::SkipWithYes,
            RciInjectMode::Browser,
        );

        let outcome = session.run().await.expect("browser inject should poll");

        let expected_url = pending_accept_url(&server.uri(), node_id(), "pending-1");
        assert_eq!(outcome.browser_url.as_deref(), Some(expected_url.as_str()));
        assert_eq!(outcome.remote_state.as_deref(), Some("consumed"));
        let received = server.received_requests().await.expect("requests");
        assert!(
            !received
                .iter()
                .any(|request| request.url.path().ends_with("/ciphertext"))
        );
        // SAFETY: env mutation is serialized by env_lock and this test runs on one thread.
        unsafe {
            std::env::remove_var("NYXID_WIZARD_NO_OPEN");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    #[allow(clippy::await_holding_lock)] // env_lock serializes env mutation across single-thread test; held across await by design
    async fn inject_browser_wizard_secret_not_in_terminal() {
        let _guard = crate::test_support::env_lock().lock().unwrap();
        // SAFETY: env mutation is serialized by env_lock and this test runs on one thread.
        unsafe {
            std::env::set_var("NYXID_WIZARD_NO_OPEN", "1");
            std::env::set_var("NYXID_RCI_BROWSER_SECRET", "sk-browser-only");
        }
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!(
                "/api/v1/nodes/{}/credentials/pending/pending-1/remote-crypto",
                node_id()
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(pending_info_json("pending-1")))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!(
                "/api/v1/nodes/{}/credentials/pending",
                node_id()
            )))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(consumed_pending_list("pending-1")),
            )
            .expect(1)
            .mount(&server)
            .await;
        let mut api = test_api(&server);
        let mut session = RciInjectSession::new(
            &mut api,
            node_id().to_string(),
            Some("pending-1".to_string()),
            None,
            RciSecretSource::BrowserOnly,
            RciFingerprintPolicy::SkipWithYes,
            RciInjectMode::Browser,
        );

        session
            .run()
            .await
            .expect("browser pending inject should complete");

        assert_no_request_body_contains(&server, "sk-browser-only").await;
        // SAFETY: env mutation is serialized by env_lock and this test runs on one thread.
        unsafe {
            std::env::remove_var("NYXID_WIZARD_NO_OPEN");
            std::env::remove_var("NYXID_RCI_BROWSER_SECRET");
        }
    }

    #[tokio::test]
    async fn inject_pending_legacy_performs_metadata_init_and_proceeds() {
        let server = MockServer::start().await;
        let keypair = nyxid_crypto::generate_node_keypair();
        let fingerprint = nyxid_crypto::rci_pubkey_fingerprint(keypair.public_key());
        mount_pending_inject_flow(&server, "pending-legacy", &keypair).await;
        let mut api = test_api(&server);
        let mut session = RciInjectSession::new(
            &mut api,
            node_id().to_string(),
            Some("pending-legacy".to_string()),
            None,
            RciSecretSource::Prompt,
            RciFingerprintPolicy::Expect(fingerprint),
            RciInjectMode::Terminal,
        )
        .with_prompt_secret_override(Zeroizing::new("Bearer sk-legacy".to_string()));

        let outcome = session
            .run()
            .await
            .expect("legacy pending inject should complete");

        assert_eq!(outcome.remote_state.as_deref(), Some("consumed"));
        let received = server.received_requests().await.expect("requests");
        assert!(received.iter().any(|request| {
            request.url.path()
                == format!(
                    "/api/v1/nodes/{}/credentials/pending/pending-legacy/remote-crypto",
                    node_id()
                )
        }));
        assert!(received.iter().any(|request| {
            request.url.path()
                == format!(
                    "/api/v1/nodes/{}/credentials/pending/pending-legacy/ciphertext",
                    node_id()
                )
        }));
        assert_no_request_body_contains(&server, "sk-legacy").await;
    }

    #[test]
    fn cli_encrypt_to_node_decrypt_interop_fixture() {
        let keypair = nyxid_crypto::generate_node_keypair();
        let pending = PendingCredentialInfoCli {
            id: "pending-interop".to_string(),
            node_id: Some("node-interop".to_string()),
            service_slug: "openai".to_string(),
            injection_method: "header".to_string(),
            field_name: "Authorization".to_string(),
            target_url: Some("https://api.openai.com/v1".to_string()),
            expires_at: "2099-01-01T00:00:00Z".to_string(),
            consumed_at: None,
            declined_at: None,
            remote_state: Some("pubkey_posted".to_string()),
            is_active: true,
        };
        let pubkey = PendingCredentialPubkeyResponseCli {
            pending_id: "pending-interop".to_string(),
            node_id: "node-interop".to_string(),
            service_slug: "openai".to_string(),
            version: nyxid_crypto::VERSION_V1.to_string(),
            node_pubkey: keypair.public_key_b64u(),
            remote_state: Some("pubkey_posted".to_string()),
            integrity_verification_opt_out: true,
        };
        validate_pubkey_response_matches_pending(&pending, &pubkey).expect("matching pubkey");

        let body = encrypt_pending_secret_for_node(
            &pending,
            &pubkey,
            &Zeroizing::new("Bearer sk-interop".to_string()),
        )
        .expect("CLI encrypt");

        let envelope = nyxid_crypto::envelope_from_encoded_parts(
            body.version,
            &body.admin_pubkey,
            &body.nonce,
            &body.ciphertext,
        )
        .expect("envelope");
        let metadata = crate::node::credentials::crypto::PendingCredentialCryptoMetadata {
            pending_id: pending.id,
            node_id: "node-interop".to_string(),
            service_slug: pending.service_slug,
            injection_method: pending.injection_method,
            field_name: pending.field_name,
            target_url: pending.target_url,
            expires_at: pending.expires_at,
            version: nyxid_crypto::VERSION_V1.to_string(),
        };
        let plaintext =
            nyxid_crypto::decrypt(&envelope, keypair.private_key(), &metadata.context())
                .expect("node decrypt");

        assert_eq!(plaintext.as_slice(), b"Bearer sk-interop");
    }
}
