//! Backend-owned Kubernetes configuration and process-lifetime metadata cache.
//! Unrelated proxy/MCP paths and kubelet credentials never use this source.
use anyhow::{Result, anyhow, ensure};
use nyxid_machine_updater::kubernetes_credentials::KubernetesCredentials;
use reqwest::{Client, Url};
use std::{path::PathBuf, sync::OnceLock, time::Duration};

pub(super) static GOOGLE_METADATA: OnceLock<Result<KubernetesCredentials, ()>> = OnceLock::new();

pub(super) struct KubernetesConnection {
    pub base: Url,
    pub client: Client,
    pub credentials: KubernetesCredentials,
}
fn invalid() -> anyhow::Error {
    anyhow!("invalid managed Kubernetes identity configuration")
}
fn optional(name: &str) -> Result<Option<String>> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(_) => Err(invalid()),
    }
}
impl KubernetesConnection {
    pub fn from_environment(request_timeout: Duration) -> Result<Self> {
        let base: Url = optional("NYXID_REMOTE_MACHINE_KUBE_API")?
            .ok_or_else(invalid)?
            .parse()
            .map_err(|_| invalid())?;
        ensure!(
            base.scheme() == "https"
                && base.host_str().is_some()
                && base.username().is_empty()
                && base.password().is_none()
                && base.query().is_none()
                && base.fragment().is_none(),
            "invalid managed Kubernetes identity configuration"
        );
        let credentials = match optional("NYXID_REMOTE_MACHINE_KUBE_AUTH_MODE")?
            .as_deref()
            .unwrap_or("token_file")
        {
            "token_file" => KubernetesCredentials::token_file(PathBuf::from(
                optional("NYXID_REMOTE_MACHINE_KUBE_TOKEN_FILE")?.ok_or_else(invalid)?,
            ))
            .map_err(|_| invalid())?,
            "google_metadata" => GOOGLE_METADATA
                .get_or_init(|| KubernetesCredentials::google_metadata().map_err(|_| ()))
                .as_ref()
                .map_err(|_| invalid())?
                .clone(),
            _ => return Err(invalid()),
        };
        let mut client = Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(request_timeout)
            .redirect(reqwest::redirect::Policy::none());
        if let Some(path) = optional("NYXID_REMOTE_MACHINE_KUBE_CA_FILE")? {
            let pem = std::fs::read(path).map_err(|_| invalid())?;
            // reqwest/rustls accepts an empty PEM input unless we explicitly
            // require at least one certificate. Invalid DER is rejected by build.
            let roots = reqwest::Certificate::from_pem_bundle(&pem).map_err(|_| invalid())?;
            ensure!(
                !roots.is_empty(),
                "invalid managed Kubernetes identity configuration"
            );
            for root in roots {
                client = client.add_root_certificate(root);
            }
        }
        Ok(Self {
            base,
            client: client.build().map_err(|_| invalid())?,
            credentials,
        })
    }
}
