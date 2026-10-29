//! Explicit migration aliases. Request headers never expand this trust set.
use super::AppConfig;
use std::borrow::Cow;

#[derive(Clone, Debug, Default)]
pub struct DomainAliases {
    pub base_urls: Vec<String>,
    pub frontend_urls: Vec<String>,
    pub cookie_domains: Vec<String>,
    pub jwt_audience: Option<String>,
}

impl DomainAliases {
    pub(super) fn from_env() -> Self {
        Self {
            base_urls: parse_origins(&std::env::var("BASE_URL_ALIASES").unwrap_or_default())
                .expect("BASE_URL_ALIASES must contain HTTPS origins only"),
            frontend_urls: parse_origins(
                &std::env::var("FRONTEND_URL_ALIASES").unwrap_or_default(),
            )
            .expect("FRONTEND_URL_ALIASES must contain HTTPS origins only"),
            cookie_domains: parse_cookie_domains(
                &std::env::var("COOKIE_DOMAIN_ALIASES").unwrap_or_default(),
            )
            .expect("COOKIE_DOMAIN_ALIASES must contain DNS cookie domains only"),
            jwt_audience: std::env::var("JWT_AUDIENCE")
                .ok()
                .filter(|s| !s.trim().is_empty()),
        }
    }
}

fn entries(raw: &str) -> Result<Vec<&str>, &'static str> {
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let entries: Vec<_> = raw.split(',').map(str::trim).collect();
    if entries.len() > 32 || entries.iter().any(|s| s.is_empty()) {
        return Err("invalid alias list");
    }
    Ok(entries)
}

pub(super) fn parse_origins(raw: &str) -> Result<Vec<String>, &'static str> {
    entries(raw)?
        .into_iter()
        .map(|s| {
            let url = url::Url::parse(s).map_err(|_| "invalid origin")?;
            let origin = url.origin().ascii_serialization();
            // Inspect the raw authority too: URL parsers normalize dot paths and
            // backslashes, which must not turn a path-bearing setting into an origin.
            let authority = s.split_once("://").ok_or("invalid origin")?.1;
            let authority = authority.strip_suffix('/').unwrap_or(authority);
            if url.scheme() != "https"
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
                || authority.contains(['/', '\\', '@', '?', '#', '%'])
                || authority
                    .bytes()
                    .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
            {
                return Err("invalid origin");
            }
            Ok(origin)
        })
        .collect()
}

fn parse_cookie_domains(raw: &str) -> Result<Vec<String>, &'static str> {
    entries(raw)?
        .into_iter()
        .map(|s| {
            let domain = s.strip_prefix('.').unwrap_or(s);
            if domain.len() > 253
                || !domain.contains('.')
                || domain.parse::<std::net::IpAddr>().is_ok()
                || domain.split('.').any(|part| {
                    part.is_empty()
                        || part.len() > 63
                        || part.starts_with('-')
                        || part.ends_with('-')
                        || !part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                })
            {
                return Err("invalid cookie domain");
            }
            Ok(s.to_ascii_lowercase())
        })
        .collect()
}

impl AppConfig {
    pub fn jwt_audience(&self) -> &str {
        self.domain_aliases
            .jwt_audience
            .as_deref()
            .unwrap_or(&self.base_url)
    }
    pub fn accepted_jwt_issuers(&self) -> Vec<&str> {
        std::iter::once(self.jwt_issuer.as_str())
            .chain(self.domain_aliases.base_urls.iter().map(String::as_str))
            .collect()
    }
    pub fn accepted_jwt_audiences(&self) -> Vec<&str> {
        std::iter::once(self.jwt_audience())
            .chain(self.domain_aliases.base_urls.iter().map(String::as_str))
            .collect()
    }
    pub fn accepts_jwt_issuer(&self, issuer: &str) -> bool {
        self.accepted_jwt_issuers().contains(&issuer)
    }
    pub fn accepted_base_urls(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.base_url.as_str())
            .chain(self.domain_aliases.base_urls.iter().map(String::as_str))
    }
    pub fn accepted_frontend_urls(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.frontend_url.as_str())
            .chain(self.domain_aliases.frontend_urls.iter().map(String::as_str))
    }
    pub fn frontend_origin_allowed(&self, origin: &str) -> bool {
        self.accepted_frontend_urls()
            .any(|s| url::Url::parse(s).is_ok_and(|u| u.origin().ascii_serialization() == origin))
    }
    /// Exact origin/prefix boundary; never accepts lookalike hosts or URL credentials.
    pub fn canonical_base_url<'a>(&self, value: &'a str) -> Cow<'a, str> {
        for alias in &self.domain_aliases.base_urls {
            if let Some(rest) = value.strip_prefix(alias)
                && (rest.is_empty() || rest.starts_with('/'))
            {
                return Cow::Owned(format!("{}{rest}", self.base_url.trim_end_matches('/')));
            }
        }
        Cow::Borrowed(value)
    }
    pub fn same_base_url(&self, left: &str, right: &str) -> bool {
        self.canonical_base_url(left) == self.canonical_base_url(right)
    }
    pub fn canonical_resources(&self, values: &[String]) -> Vec<String> {
        values
            .iter()
            .map(|s| self.canonical_base_url(s).into_owned())
            .collect()
    }
    pub fn resources_equal(&self, left: &[String], right: &[String]) -> bool {
        left.len() == right.len()
            && left
                .iter()
                .zip(right)
                .all(|(a, b)| self.same_base_url(a, b))
    }
    pub fn trusted_return_to(&self, value: &str) -> bool {
        self.accepted_frontend_urls()
            .chain(self.accepted_base_urls())
            .any(|s| value.starts_with(&format!("{}/", s.trim_end_matches('/'))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_alias_origins_are_explicit_https_origins() {
        assert!(parse_origins(" \t ").unwrap().is_empty());
        assert_eq!(
            parse_origins("https://old.example, https://other.example/").unwrap(),
            ["https://old.example", "https://other.example"]
        );
        assert_eq!(
            parse_origins("https://OLD.example:443/").unwrap(),
            ["https://old.example"]
        );
        for bad in [
            "http://old.example",
            "https://old.example/path",
            "https://old.example//",
            "https://old.example?x",
            "https://old.example#x",
            "https://user@old.example",
            "https://old.example,",
            "https://old.example/../",
            "https://old.example\\evil",
        ] {
            assert!(parse_origins(bad).is_err(), "{bad}");
        }
        assert!(parse_origins(&["https://old.example"; 33].join(",")).is_err());
    }

    #[test]
    fn domain_alias_cookie_domains_cannot_inject_attributes() {
        assert_eq!(
            parse_cookie_domains(".OLD.example, api.example.com").unwrap(),
            [".old.example", "api.example.com"]
        );
        for bad in [
            "https://old.example",
            "old.example:443",
            "old.example; Secure",
            "*.old.example",
            ".",
            "localhost",
            "127.0.0.1",
            "a..example",
            "-a.example",
        ] {
            assert!(parse_cookie_domains(bad).is_err(), "{bad}");
        }
        assert!(parse_cookie_domains("").unwrap().is_empty());
    }

    #[test]
    fn domain_alias_defaults_preserve_single_origin_and_audience() {
        let config = crate::test_utils::test_app_config();
        assert_eq!(config.jwt_audience(), config.base_url);
        assert_eq!(config.accepted_jwt_issuers(), [config.jwt_issuer.as_str()]);
        assert_eq!(config.accepted_jwt_audiences(), [config.base_url.as_str()]);
        assert_eq!(
            config.accepted_base_urls().collect::<Vec<_>>(),
            [config.base_url.as_str()]
        );
        assert_eq!(
            config.canonical_base_url("https://old.example/mcp"),
            "https://old.example/mcp"
        );
        assert!(!config.trusted_return_to("https://old.example/login"));
    }

    #[test]
    fn domain_alias_prefixes_do_not_expand_to_unconfigured_hosts() {
        let mut config = crate::test_utils::test_app_config();
        config.base_url = "https://new.example".into();
        config.domain_aliases.base_urls = vec!["https://old.example".into()];
        config.domain_aliases.frontend_urls = vec!["https://old-app.example".into()];
        assert_eq!(
            config.canonical_base_url("https://old.example/api/v1/proxy/s/test"),
            "https://new.example/api/v1/proxy/s/test"
        );
        for value in [
            "https://old.example.evil/mcp",
            "https://old.example@evil/mcp",
            "https://old.example:444/mcp",
        ] {
            assert_eq!(config.canonical_base_url(value), value);
            assert!(!config.trusted_return_to(value));
        }
        assert!(config.trusted_return_to("https://old-app.example/login"));
        assert!(config.frontend_origin_allowed("https://old-app.example"));
        assert!(!config.accepts_jwt_issuer("https://old-app.example"));
    }
}
