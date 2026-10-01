//! Safe, shared network diagnostics. Never include request/response payloads.
use serde::Serialize;
use std::{error::Error, sync::LazyLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Config,
    Connect,
    Proxy,
    Tls,
    Request,
    Response,
    Validation,
    Storage,
}

#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub stage: Stage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_error_code: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    pub causes: Vec<String>,
    pub hint: String,
}

impl Diagnostic {
    pub fn new(
        stage: Stage,
        endpoint: Option<&str>,
        http_status: Option<u16>,
        cause: &str,
    ) -> Self {
        let cause = sanitize(cause).chars().take(300).collect::<String>();
        let hint = match stage {
            Stage::Config => ["NYXID_CA_CERT", "SSL_CERT_FILE", "SSL_CERT_DIR"]
                .into_iter()
                .find(|name| cause.contains(name))
                .map(|name| format!("Fix or unset {name}, then retry."))
                .unwrap_or_else(|| "Check the network and TLS configuration, then retry.".into()),
            Stage::Tls => {
                "Check the server TLS configuration and any proxy on this connection.".into()
            }
            Stage::Proxy => {
                "Check HTTPS_PROXY and NO_PROXY and whether the proxy permits CONNECT to this host."
                    .into()
            }
            Stage::Connect | Stage::Request => {
                "Check network/proxy reachability and retry when the destination is reachable."
                    .into()
            }
            Stage::Response => {
                "Check the requested endpoint and the service status, then retry.".into()
            }
            Stage::Validation => {
                "Check the server configuration; its response contains an invalid value.".into()
            }
            Stage::Storage => {
                "Check permissions and available space in the local profile directory, then retry."
                    .into()
            }
        };
        Self {
            stage,
            timeout: None,
            http_status,
            server_error_code: None,
            endpoint: endpoint.and_then(sanitize_endpoint),
            causes: vec![cause],
            hint,
        }
    }

    pub fn reqwest(error: &reqwest::Error, endpoint: Option<&str>, status: Option<u16>) -> Self {
        let mut chain: Vec<&(dyn Error + 'static)> = Vec::new();
        let mut current: Option<&(dyn Error + 'static)> = Some(error);
        while let Some(err) = current {
            chain.push(err);
            if chain.len() == 32 {
                break;
            }
            // io::Error::source can skip its contained error. hyper-rustls
            // wraps tokio-rustls's IO error again, so inspect get_ref first.
            current = match err
                .downcast_ref::<std::io::Error>()
                .and_then(|e| e.get_ref())
            {
                Some(inner) => Some(inner),
                None => err.source(),
            };
        }
        let tls = chain.iter().find_map(|e| e.downcast_ref::<rustls::Error>());
        // reqwest 0.12.28 delegates CONNECT to hyper-util's private TunnelError.
        // Its stable Display prefix is the only exposed discriminator; status
        // codes (except 407's message) are not retained by that error type.
        let proxy = chain
            .iter()
            .any(|e| e.to_string().starts_with("tunnel error:"));
        let stage = if tls.is_some() {
            Stage::Tls
        } else if proxy {
            Stage::Proxy
        } else if error.is_connect() {
            Stage::Connect
        } else if error.is_decode() || error.is_body() || error.status().is_some() {
            Stage::Response
        } else if error.is_builder() {
            Stage::Config
        } else {
            Stage::Request
        };
        let mut result = Self::new(
            stage,
            endpoint.or_else(|| error.url().map(|u| u.as_str())),
            status.or_else(|| error.status().map(|s| s.as_u16())),
            "request failed",
        );
        result.timeout = error.is_timeout().then_some(if error.is_connect() {
            "connect"
        } else {
            "request"
        });
        if let Some(tls) = tls {
            result.hint = tls_hint(tls).into();
        }
        result.causes = chain.into_iter().map(cause_text).collect();
        result.causes.dedup();
        result.causes.truncate(8);
        result
    }

    pub fn for_login(mut self) -> Self {
        self.hint = match self.stage {
            Stage::Response => "Check that --base-url points to a NyxID API and that the server is healthy.".into(),
            Stage::Validation => "The server returned an invalid verification URL; check its frontend URL configuration.".into(),
            Stage::Tls if self.hint == HOST_HINT => "The certificate does not match the host; check --base-url and whether a proxy intercepts this host.".into(),
            _ => self.hint,
        };
        self
    }

    pub fn for_github(mut self) -> Self {
        if self.stage == Stage::Response {
            self.hint = "Check GitHub API status and rate limits, then retry.".into();
        }
        self
    }

    pub fn from_anyhow(error: &anyhow::Error) -> Option<Self> {
        if let Some(config) = error
            .chain()
            .find_map(|e| e.downcast_ref::<crate::tls::TlsError>())
        {
            return Some(Self::new(Stage::Config, None, None, &config.to_string()));
        }
        error
            .chain()
            .find_map(|e| e.downcast_ref::<reqwest::Error>())
            .map(|e| Self::reqwest(e, None, None))
    }

    pub fn text(&self) -> String {
        let stage = serde_json::to_value(self.stage).expect("stage serializes");
        let mut lines = vec![format!(
            "  stage: {}",
            stage.as_str().expect("stage string")
        )];
        if let Some(timeout) = self.timeout {
            lines.push(format!("  timeout: {timeout}"));
        }
        if let Some(status) = self.http_status {
            lines.push(format!("  status: {status}"));
        }
        if let Some(code) = self.server_error_code {
            lines.push(format!("  server_error_code: {code}"));
        }
        if let Some(endpoint) = &self.endpoint {
            lines.push(format!("  endpoint: {endpoint}"));
        }
        lines.extend(self.causes.iter().map(|cause| format!("  cause: {cause}")));
        lines.push(format!("  hint: {}", self.hint));
        lines.join("\n")
    }
}

pub fn cause_text(error: &(dyn Error + 'static)) -> String {
    error_text(error).chars().take(300).collect()
}

pub fn error_text(error: &(dyn Error + 'static)) -> String {
    // serde errors can quote arbitrary server strings, including credentials.
    let message = if let Some(json) = error.downcast_ref::<serde_json::Error>() {
        format!(
            "invalid JSON response at line {}, column {}",
            json.line(),
            json.column()
        )
    } else {
        error.to_string()
    };
    sanitize(&message)
}

const HOST_HINT: &str = "The certificate does not match the requested host; check the URL and whether a proxy intercepts this host.";
fn tls_hint(error: &rustls::Error) -> &'static str {
    use rustls::CertificateError::*;
    match error {
        rustls::Error::InvalidCertificate(UnknownIssuer | BadSignature) => {
            "Check NYXID_CA_CERT or SSL_CERT_FILE for the CA used by your server or TLS-inspecting proxy."
        }
        rustls::Error::InvalidCertificate(NotValidForName | NotValidForNameContext { .. }) => {
            HOST_HINT
        }
        rustls::Error::InvalidCertificate(Expired | ExpiredContext { .. }) => {
            "The certificate has expired; check the server certificate and system clock."
        }
        rustls::Error::InvalidCertificate(NotValidYet | NotValidYetContext { .. }) => {
            "The certificate is not yet valid; check the server certificate and system clock."
        }
        _ => "Check the server TLS configuration and any proxy on this connection.",
    }
}

/// NO_PROXY contains host patterns rather than proxy credentials.
pub fn bypass_list(raw: &str) -> String {
    raw.chars().filter(|c| !c.is_control()).take(300).collect()
}

pub fn sanitize_endpoint(raw: &str) -> Option<String> {
    let mut url = url::Url::parse(raw).ok()?;
    url.host_str()?;
    url.set_username("").ok()?;
    url.set_password(None).ok()?;
    url.set_query(None);
    url.set_fragment(None);
    Some(url.to_string())
}

pub fn proxy_origin(raw: &str) -> String {
    let raw = if raw.contains("://") {
        raw.to_owned()
    } else {
        format!("http://{raw}")
    };
    url::Url::parse(&raw)
        .ok()
        .filter(|u| u.host_str().is_some())
        .map(|u| {
            format!(
                "{}://{}{}",
                u.scheme(),
                u.host_str().unwrap_or_default(),
                u.port().map(|p| format!(":{p}")).unwrap_or_default()
            )
        })
        .unwrap_or_else(|| "set (invalid proxy URL; value redacted)".into())
}

pub fn sanitize(raw: &str) -> String {
    static URLS: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r#"[a-zA-Z][a-zA-Z0-9+.-]*://[^\s<>\"']+"#).expect("URL pattern")
    });
    static SECRETS: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?i)(authorization\s*:|bearer\s+|password\s*[:=]|access_token|refresh_token|device_code|poll_secret|login_code|nyx_adc|nyx_akl|nyxid_ag_)").expect("secret pattern")
    });
    let clean = URLS.replace_all(raw, |caps: &regex::Captures<'_>| {
        sanitize_endpoint(&caps[0]).unwrap_or_else(|| "[redacted URL]".into())
    });
    if SECRETS.is_match(&clean) {
        return "[redacted sensitive error detail]".into();
    }
    clean
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

#[cfg(test)]
mod tests;
