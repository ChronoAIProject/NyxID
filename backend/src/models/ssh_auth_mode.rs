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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_str_values() {
        assert_eq!(SshAuthMode::Cert.as_str(), "cert");
        assert_eq!(SshAuthMode::NodeKey.as_str(), "node_key");
        assert_eq!(SshAuthMode::ProxyOnly.as_str(), "proxy_only");
    }

    #[test]
    fn display_matches_as_str() {
        assert_eq!(format!("{}", SshAuthMode::Cert), "cert");
        assert_eq!(format!("{}", SshAuthMode::NodeKey), "node_key");
        assert_eq!(format!("{}", SshAuthMode::ProxyOnly), "proxy_only");
    }

    #[test]
    fn from_str_valid() {
        assert_eq!("cert".parse::<SshAuthMode>().unwrap(), SshAuthMode::Cert);
        assert_eq!(
            "node_key".parse::<SshAuthMode>().unwrap(),
            SshAuthMode::NodeKey
        );
        assert_eq!(
            "proxy_only".parse::<SshAuthMode>().unwrap(),
            SshAuthMode::ProxyOnly
        );
    }

    #[test]
    fn from_str_invalid() {
        assert!("invalid".parse::<SshAuthMode>().is_err());
    }

    #[test]
    fn from_certificate_auth_enabled() {
        assert_eq!(
            SshAuthMode::from_certificate_auth_enabled(true),
            SshAuthMode::Cert
        );
        assert_eq!(
            SshAuthMode::from_certificate_auth_enabled(false),
            SshAuthMode::ProxyOnly
        );
    }

    #[test]
    fn certificate_auth_enabled_method() {
        assert!(SshAuthMode::Cert.certificate_auth_enabled());
        assert!(!SshAuthMode::NodeKey.certificate_auth_enabled());
        assert!(!SshAuthMode::ProxyOnly.certificate_auth_enabled());
    }

    #[test]
    fn default_is_proxy_only() {
        assert_eq!(SshAuthMode::default(), SshAuthMode::ProxyOnly);
        assert_eq!(default_ssh_auth_mode(), SshAuthMode::ProxyOnly);
    }

    #[test]
    fn serde_roundtrip() {
        for mode in [
            SshAuthMode::Cert,
            SshAuthMode::NodeKey,
            SshAuthMode::ProxyOnly,
        ] {
            let json = serde_json::to_string(&mode).unwrap();
            let back: SshAuthMode = serde_json::from_str(&json).unwrap();
            assert_eq!(back, mode);
        }
    }

    #[test]
    fn serde_snake_case_names() {
        assert_eq!(
            serde_json::to_string(&SshAuthMode::Cert).unwrap(),
            "\"cert\""
        );
        assert_eq!(
            serde_json::to_string(&SshAuthMode::NodeKey).unwrap(),
            "\"node_key\""
        );
        assert_eq!(
            serde_json::to_string(&SshAuthMode::ProxyOnly).unwrap(),
            "\"proxy_only\""
        );
    }
}
