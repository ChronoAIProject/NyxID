use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::errors::AppError;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SshAuthMode {
    Cert,
    NodeKey,
    #[default]
    ProxyOnly,
}

impl SshAuthMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cert => "cert",
            Self::NodeKey => "node_key",
            Self::ProxyOnly => "proxy_only",
        }
    }

    pub fn from_certificate_auth_enabled(enabled: bool) -> Self {
        if enabled { Self::Cert } else { Self::ProxyOnly }
    }

    pub fn certificate_auth_enabled(self) -> bool {
        matches!(self, Self::Cert)
    }
}

impl fmt::Display for SshAuthMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SshAuthMode {
    type Err = AppError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "cert" => Ok(Self::Cert),
            "node_key" => Ok(Self::NodeKey),
            "proxy_only" => Ok(Self::ProxyOnly),
            other => Err(AppError::ValidationError(format!(
                "Invalid ssh_auth_mode '{other}'. Valid: cert, node_key, proxy_only"
            ))),
        }
    }
}

pub fn default_ssh_auth_mode() -> SshAuthMode {
    SshAuthMode::ProxyOnly
}
