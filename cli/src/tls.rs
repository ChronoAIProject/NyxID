//! Process-wide TLS trust for HTTP and WebSocket clients.
use std::{
    ffi::OsString,
    path::PathBuf,
    sync::{Arc, LazyLock},
    time::Instant,
};

use pki_types::{CertificateDer, pem::PemObject};
use rustls::{ClientConfig, RootCertStore};
use serde::Serialize;

pub(crate) mod environment;
mod verifier;

pub const ENV_HELP: &str = "Network / TLS:\n  NYXID_CA_CERT   Additional CA certificates (PEM file)\n  SSL_CERT_FILE  System CA bundle override\n  SSL_CERT_DIR   System CA directories (platform path separator)\n  HTTPS_PROXY / NO_PROXY  HTTP proxy and bypass hosts (WSS connects directly)\n  Run nyxid doctor for trust sources and network diagnostics.";

#[derive(Clone, Debug, thiserror::Error)]
#[error("{0}")]
pub struct TlsError(pub String);

#[derive(Default)]
pub(crate) struct Sources {
    pub ca_file: Option<PathBuf>,
    pub system_file: Option<PathBuf>,
    pub system_dirs: Option<OsString>,
}
impl Sources {
    fn from_env() -> Self {
        Self {
            ca_file: environment::nonempty_env("NYXID_CA_CERT").map(PathBuf::from),
            system_file: environment::nonempty_env("SSL_CERT_FILE").map(PathBuf::from),
            system_dirs: environment::nonempty_env("SSL_CERT_DIR"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct TrustSource {
    pub name: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub certificates: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub struct Trust {
    pub sources: Vec<TrustSource>,
    explicit_load_ms: f64,
    platform: Option<Arc<verifier::LazyPlatform>>,
    config: Result<Arc<ClientConfig>, TlsError>,
}

pub struct TrustReport {
    pub sources: Vec<TrustSource>,
    pub native_load_ms: f64,
}

impl Trust {
    /// Doctor explicitly loads the OS store even if no connection needed it.
    pub fn report(&self) -> TrustReport {
        let mut sources = self.sources.clone();
        let native_load_ms = if let Some(platform) = &self.platform {
            let native = platform.get();
            sources.push(TrustSource {
                name: "OS store",
                path: None,
                certificates: native.certificates,
                error: None,
            });
            native.load_ms
        } else {
            self.explicit_load_ms
        };
        TrustReport {
            sources,
            native_load_ms,
        }
    }
}

static TRUST: LazyLock<Trust> =
    LazyLock::new(|| load(&Sources::from_env(), rustls_native_certs::load_native_certs));

pub fn trust() -> &'static Trust {
    &TRUST
}

pub fn shared_config() -> Result<Arc<ClientConfig>, TlsError> {
    TRUST.config.clone()
}

pub fn client_builder() -> Result<reqwest::ClientBuilder, TlsError> {
    Ok(http_builder(shared_config()?.as_ref()))
}

fn http_builder(config: &ClientConfig) -> reqwest::ClientBuilder {
    // reqwest 0.12 uses BuiltRustls as-is. Without its http2 feature, both
    // HTTP and WSS use HTTP/1.1 without ALPN.
    reqwest::Client::builder().use_preconfigured_tls(config.clone())
}

// Inject both paths and the platform loader so tests never mutate process env.
pub(crate) fn load(
    sources: &Sources,
    platform: impl FnOnce() -> rustls_native_certs::CertificateResult + Send + 'static,
) -> Trust {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let mut report = vec![TrustSource {
        name: "Bundled Mozilla",
        path: None,
        certificates: roots.len(),
        error: None,
    }];
    let started = Instant::now();
    let system_file = sources
        .system_file
        .as_ref()
        .filter(|p| !p.as_os_str().is_empty());
    let system_dirs = sources.system_dirs.as_ref().filter(|p| !p.is_empty());
    {
        if let Some(path) = system_file {
            let loaded = rustls_native_certs::load_certs_from_paths(Some(path), None);
            let (count, _) = roots.add_parsable_certificates(loaded.certs);
            report.push(system_source(
                "SSL_CERT_FILE",
                path.display().to_string(),
                count,
            ));
        }
        if let Some(paths) = system_dirs {
            let mut count = 0;
            for path in std::env::split_paths(paths).filter(|p| !p.as_os_str().is_empty()) {
                let loaded = rustls_native_certs::load_certs_from_paths(None, Some(&path));
                count += roots.add_parsable_certificates(loaded.certs).0;
            }
            report.push(system_source(
                "SSL_CERT_DIR",
                paths.to_string_lossy().into_owned(),
                count,
            ));
        }
    }
    let explicit_load_ms = started.elapsed().as_secs_f64() * 1000.0;
    if let Some(path) = sources
        .ca_file
        .as_ref()
        .filter(|p| !p.as_os_str().is_empty())
    {
        let result = (|| {
            let data = std::fs::read(path).map_err(|e| format!("cannot read PEM file: {e}"))?;
            let certs = CertificateDer::pem_slice_iter(&data)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| "invalid PEM certificates".to_string())?;
            if certs.is_empty() {
                return Err("no PEM certificates found".to_string());
            }
            let count = certs.len();
            for cert in certs {
                roots
                    .add(cert)
                    .map_err(|_| "invalid DER certificate".to_string())?;
            }
            Ok(count)
        })();
        let (certificates, error) = match result {
            Ok(count) => (count, None),
            Err(message) => (
                0,
                Some(format!("NYXID_CA_CERT ({}): {message}", path.display())),
            ),
        };
        report.push(TrustSource {
            name: "NYXID_CA_CERT",
            path: Some(path.display().to_string()),
            certificates,
            error,
        });
    }
    let platform = (system_file.is_none() && system_dirs.is_none())
        .then(|| Arc::new(verifier::LazyPlatform::new(roots.clone(), platform)));
    let config = if let Some(error) = report.iter().find_map(|s| s.error.as_ref()) {
        Err(TlsError(error.clone()))
    } else {
        Ok(Arc::new(
            ClientConfig::builder_with_provider(Arc::new(
                rustls::crypto::aws_lc_rs::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .expect("supported TLS versions")
            .dangerous()
            .with_custom_certificate_verifier(verifier::build(roots, platform.clone()))
            .with_no_client_auth(),
        ))
    };
    Trust {
        sources: report,
        explicit_load_ms,
        platform,
        config,
    }
}

fn system_source(name: &'static str, path: String, count: usize) -> TrustSource {
    let error = (count == 0).then(|| format!("{name} ({path}): no usable CA certificates"));
    TrustSource {
        name,
        path: Some(path),
        certificates: count,
        error,
    }
}

#[cfg(test)]
pub(crate) mod tests;
