use anyhow::{Result, bail, ensure};
use clap::Args;

use crate::cli::LoginArgs;

#[derive(Args, Default)]
pub struct LoginHintArgs {
    /// Suggested approval choice; --agent-key enforces restricted delivery
    #[arg(long, value_parser = ["full", "agent"], help_heading = "Approval preferences")]
    pub login_type: Option<String>,
    /// Suggest choosing an existing Agent Key or creating a new one
    #[arg(long, value_parser = ["existing", "new"], help_heading = "Approval preferences")]
    pub key_source: Option<String>,
    /// Suggested new Agent Key name (1–64 characters)
    #[arg(long, help_heading = "Approval preferences")]
    pub key_name: Option<String>,
    /// Requested NyxID scopes, repeated or comma-separated; editable at approval
    #[arg(
        long,
        visible_alias = "permissions",
        value_delimiter = ',',
        help_heading = "Approval preferences"
    )]
    pub scopes: Vec<String>,
    /// Requested catalog service slug, repeated or comma-separated
    #[arg(
        long = "service",
        visible_alias = "services",
        value_delimiter = ',',
        help_heading = "Approval preferences"
    )]
    pub services: Vec<String>,
    /// Requested catalog-slug::provider-scope, repeated or comma-separated; does not narrow existing provider grants
    #[arg(
        long = "service-permission",
        visible_alias = "service-permissions",
        value_delimiter = ',',
        help_heading = "Approval preferences"
    )]
    pub service_permissions: Vec<String>,
    /// Suggested new-key lifetime in days, or none
    #[arg(long, value_parser = ["7", "30", "90", "365", "none"], help_heading = "Approval preferences")]
    pub expiry_days: Option<String>,
    /// Suggested Agent Key platform
    #[arg(long, value_parser = ["generic", "codex", "claude-code", "openclaw"], help_heading = "Approval preferences")]
    pub platform: Option<String>,
}

impl LoginHintArgs {
    pub fn is_empty(&self) -> bool {
        self.login_type.is_none()
            && self.key_source.is_none()
            && self.key_name.is_none()
            && self.scopes.is_empty()
            && self.services.is_empty()
            && self.service_permissions.is_empty()
            && self.expiry_days.is_none()
            && self.platform.is_none()
    }

    pub fn validate_mode(&self, args: &LoginArgs) -> Result<()> {
        if self.is_empty() {
            return Ok(());
        }
        ensure!(
            !args.password && !args.callback && args.code.is_none() && args.command.is_none(),
            "Approval preferences require a new device or Agent Key login; they cannot be used with --password, --callback, --code, or login resume."
        );
        Ok(())
    }

    pub fn query_pairs(&self, agent_key: bool) -> Result<Vec<(&'static str, String)>> {
        if self.is_empty() {
            return Ok(Vec::new());
        }
        let mode = self.login_type.as_deref().unwrap_or("agent");
        ensure!(matches!(mode, "full" | "agent"), "Invalid --login-type.");
        ensure!(
            !agent_key || mode == "agent",
            "--agent-key cannot request --login-type full."
        );
        if mode == "full"
            && (self.key_source.is_some()
                || self.key_name.is_some()
                || !self.scopes.is_empty()
                || !self.services.is_empty()
                || !self.service_permissions.is_empty()
                || self.expiry_days.is_some()
                || self.platform.is_some())
        {
            bail!(
                "Scope and key preferences require --login-type agent (the default when preferences are supplied)."
            );
        }
        let mut pairs = vec![("login_type", mode.to_owned())];
        for (name, value, choices) in [
            ("key_source", &self.key_source, &["existing", "new"][..]),
            (
                "expiry_days",
                &self.expiry_days,
                &["7", "30", "90", "365", "none"][..],
            ),
            (
                "platform",
                &self.platform,
                &["generic", "codex", "claude-code", "openclaw"][..],
            ),
        ] {
            if let Some(value) = value {
                ensure!(choices.contains(&value.as_str()), "Invalid {name}.");
                pairs.push((name, value.clone()));
            }
        }
        if let Some(name) = &self.key_name {
            ensure!(
                !name.trim().is_empty() && text_len(name) <= 64 && !has_controls(name),
                "--key-name must contain 1–64 characters without control characters."
            );
            pairs.push(("key_name", name.trim().to_owned()));
        }
        for (name, values, limit) in [
            ("permissions", &self.scopes, 1024),
            ("services", &self.services, 4096),
            ("service_permissions", &self.service_permissions, 8192),
        ] {
            if values.is_empty() {
                continue;
            }
            let joined = values.join(",");
            ensure!(
                text_len(&joined) <= limit && !has_controls(&joined),
                "{name} exceeds its {limit}-character limit or contains control characters."
            );
            let mut normalized = Vec::new();
            for value in joined.split(',').map(str::trim) {
                let valid = match name {
                    "permissions" => [
                        "read",
                        "write",
                        "admin",
                        "openid",
                        "profile",
                        "email",
                        "services:read",
                        "services:write",
                        "proxy",
                    ]
                    .contains(&value),
                    "services" => valid_slug(value),
                    _ => valid_service_permission(value),
                };
                ensure!(
                    valid,
                    "Invalid {name} value. See nyxid login --help and the device-login specification."
                );
                if !normalized.contains(&value) {
                    normalized.push(value);
                }
            }
            pairs.push((name, normalized.join(",")));
        }
        Ok(pairs)
    }
}

fn text_len(value: &str) -> usize {
    // Match the browser parser's JavaScript string-length bounds.
    value.encode_utf16().count()
}

fn has_controls(value: &str) -> bool {
    value.bytes().any(|b| b < 32 || b == 127)
}

fn valid_slug(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        && value.as_bytes()[0].is_ascii_alphanumeric()
}

fn valid_service_permission(value: &str) -> bool {
    let Some((slug, scope)) = value.split_once("::") else {
        return false;
    };
    valid_slug(slug) && !scope.is_empty() && !scope.contains("::") && text_len(value) <= 2048
}

pub fn approval_url(
    verification: &url::Url,
    user_code: &str,
    pairs: &[(&str, String)],
) -> Result<url::Url> {
    let mut url = verification.clone();
    url.query_pairs_mut()
        .clear()
        .append_pair("user_code", user_code)
        .extend_pairs(pairs.iter().map(|(name, value)| (*name, value.as_str())));
    ensure!(
        url.query().unwrap_or_default().len() < 49152,
        "The approval link exceeds the browser query limit."
    );
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Commands};
    use clap::Parser;

    fn parse(flags: &[&str]) -> LoginArgs {
        let cli = Cli::try_parse_from(["nyxid", "login"].into_iter().chain(flags.iter().copied()))
            .unwrap();
        let Commands::Login(args) = cli.command else {
            panic!("login expected")
        };
        args
    }

    #[test]
    fn scoped_preferences_encode_once_and_preserve_server_route() {
        let args = parse(&[
            "--device",
            "--scopes",
            "read,proxy",
            "--scopes",
            "read",
            "--service",
            "api-google-gmail",
            "--service-permission",
            "api-google-gmail::https://www.googleapis.com/auth/gmail.readonly",
            "--key-source",
            "new",
            "--key-name",
            "Mail + code & tools",
            "--expiry-days",
            "30",
            "--platform",
            "codex",
        ]);
        args.hints.validate_mode(&args).unwrap();
        let pairs = args.hints.query_pairs(args.agent_key).unwrap();
        let base = url::Url::parse("https://nyx.example/login/device").unwrap();
        let url = approval_url(&base, "ABCD-EFGH", &pairs).unwrap();
        let query: std::collections::HashMap<_, _> = url.query_pairs().collect();
        assert_eq!(url.path(), "/login/device");
        assert_eq!(query["login_type"], "agent");
        assert_eq!(query["permissions"], "read,proxy");
        assert_eq!(query["key_name"], "Mail + code & tools");
        assert_eq!(
            query["service_permissions"],
            "api-google-gmail::https://www.googleapis.com/auth/gmail.readonly"
        );
        assert_eq!(query["user_code"], "ABCD-EFGH");
    }

    #[test]
    fn preferences_reject_incompatible_flows_and_ambiguous_scope_names() {
        for flags in [
            vec!["--agent-key", "--login-type", "full"],
            vec!["--device", "--login-type", "full", "--scopes", "read"],
            vec!["--scopes", "read,,proxy"],
            vec!["--scopes", "root"],
            vec!["--service", "../account"],
            vec!["--service-permission", "repo"],
            vec!["--service-permission", "github::repo::write"],
            vec!["--key-name", "\nname"],
            vec!["--password", "--scopes", "read"],
            vec!["--callback", "--scopes", "read"],
            vec!["--code", "ABCD-EFGH", "--scopes", "read"],
            vec!["--scopes", "read", "resume", "request"],
        ] {
            let args = parse(&flags);
            assert!(
                args.hints
                    .validate_mode(&args)
                    .and_then(|_| args.hints.query_pairs(args.agent_key))
                    .is_err(),
                "accepted {flags:?}"
            );
        }
    }

    #[test]
    fn limits_match_browser_utf16_and_combined_list_bounds() {
        let mut hints = LoginHintArgs {
            key_name: Some("🙂".repeat(32)),
            ..Default::default()
        };
        assert!(hints.query_pairs(true).is_ok());
        hints.key_name = Some("🙂".repeat(33));
        assert!(hints.query_pairs(true).is_err());
        hints.key_name = None;
        hints.service_permissions = vec![format!("gmail::{}", "a".repeat(2042))];
        assert!(hints.query_pairs(true).is_err());
        hints.service_permissions = vec![format!("gmail::{}", "a".repeat(2041))];
        assert!(hints.query_pairs(true).is_ok());
        hints.service_permissions = vec![hints.service_permissions[0].clone(); 5];
        assert!(hints.query_pairs(true).is_err());
    }
}
