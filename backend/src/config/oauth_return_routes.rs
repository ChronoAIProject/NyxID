use std::collections::{BTreeMap, btree_map::Entry};
use std::fmt;
use std::marker::PhantomData;

use serde::{
    Deserialize, Deserializer,
    de::{MapAccess, Visitor},
};

const MAX_CONFIG_BYTES: usize = 65_536;
const MAX_SERVICES: usize = 128;
const MAX_PAGES: usize = 64;
const MAX_URL_BYTES: usize = 2_048;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OAuthReturnRoutes {
    default_url: String,
    #[serde(default, deserialize_with = "unique_routes")]
    services: BTreeMap<String, ServiceReturnRoutes>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ServiceReturnRoutes {
    default_url: Option<String>,
    #[serde(default, deserialize_with = "unique_routes")]
    pages: BTreeMap<String, String>,
}

fn unique_routes<'de, D, T>(deserializer: D) -> Result<BTreeMap<String, T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct UniqueRoutes<T>(PhantomData<T>);
    impl<'de, T: Deserialize<'de>> Visitor<'de> for UniqueRoutes<T> {
        type Value = BTreeMap<String, T>;
        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a map with unique route names")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut routes = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, T>()? {
                match routes.entry(key) {
                    Entry::Vacant(entry) => {
                        entry.insert(value);
                    }
                    Entry::Occupied(_) => {
                        return Err(serde::de::Error::custom("duplicate route name"));
                    }
                }
            }
            Ok(routes)
        }
    }
    deserializer.deserialize_map(UniqueRoutes(PhantomData))
}

impl fmt::Debug for OAuthReturnRoutes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuthReturnRoutes")
            .field("service_count", &self.services.len())
            .finish_non_exhaustive()
    }
}

impl OAuthReturnRoutes {
    pub fn parse(raw: &str) -> Result<Self, &'static str> {
        if raw.len() > MAX_CONFIG_BYTES {
            return Err("OAUTH_RETURN_ROUTES exceeds 65536 bytes");
        }
        let routes: Self = serde_json::from_str(raw)
            .map_err(|_| "OAUTH_RETURN_ROUTES must match the documented JSON schema")?;
        validate_url(&routes.default_url)?;
        if routes.services.len() > MAX_SERVICES {
            return Err("OAUTH_RETURN_ROUTES supports at most 128 services");
        }
        for (slug, service) in &routes.services {
            validate_name(slug)?;
            if let Some(url) = &service.default_url {
                validate_url(url)?;
            }
            if service.pages.len() > MAX_PAGES {
                return Err("OAUTH_RETURN_ROUTES supports at most 64 pages per service");
            }
            for (page, url) in &service.pages {
                validate_name(page)?;
                if page == "default" {
                    return Err("configure default_url instead of a page named default");
                }
                validate_url(url)?;
            }
        }
        Ok(routes)
    }

    pub fn resolve(&self, service_slug: &str, page: &str) -> Result<&str, &'static str> {
        validate_name(page)?;
        let service = self.services.get(service_slug);
        Ok(service
            .and_then(|service| service.pages.get(page).or(service.default_url.as_ref()))
            .unwrap_or(&self.default_url))
    }
}

fn validate_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty()
        || name.len() > 64
        || !name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
    {
        return Err("return route names must be 1-64 lowercase letters, digits, '-' or '_'");
    }
    Ok(())
}

fn validate_url(raw: &str) -> Result<(), &'static str> {
    if raw.len() > MAX_URL_BYTES
        || raw.trim() != raw
        || raw.contains('\\')
        || raw.chars().any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return Err("configured return URL must be bounded and contain no whitespace or controls");
    }
    let parsed = url::Url::parse(raw).map_err(|_| "configured return URL must be absolute")?;
    let scheme_allowed = parsed.scheme() == "https"
        || (parsed.scheme() == "http"
            && matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")));
    if !scheme_allowed
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        return Err(
            "configured return URL requires HTTPS or HTTP loopback, with no userinfo or fragment",
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_page_then_service_default_then_global_default() {
        let routes = OAuthReturnRoutes::parse(
            r#"{
                "default_url":"https://app.example/dashboard",
                "services":{
                    "api-google":{
                        "default_url":"https://app.example/keys",
                        "pages":{"onboarding":"https://app.example/onboarding?step=workspace"}
                    },
                    "api-github":{"pages":{}}
                }
            }"#,
        )
        .unwrap();
        assert_eq!(
            routes.resolve("api-google", "onboarding").unwrap(),
            "https://app.example/onboarding?step=workspace"
        );
        for page in ["default", "unknown"] {
            assert_eq!(
                routes.resolve("api-google", page).unwrap(),
                "https://app.example/keys"
            );
            assert_eq!(
                routes.resolve("api-github", page).unwrap(),
                "https://app.example/dashboard"
            );
            assert_eq!(
                routes.resolve("another-catalog-service", page).unwrap(),
                "https://app.example/dashboard"
            );
        }
        for page in [
            "",
            "https://evil.example",
            "../onboarding",
            "*",
            "Onboarding",
        ] {
            assert!(routes.resolve("api-google", page).is_err());
        }
    }

    #[test]
    fn only_explicit_loopback_http_is_allowed() {
        for url in [
            "http://127.0.0.1:3003/temp",
            "http://localhost:3003/temp",
            "http://[::1]:3003/temp",
            "https://app.example/return?step=1",
        ] {
            let raw = serde_json::json!({"default_url":url}).to_string();
            assert!(OAuthReturnRoutes::parse(&raw).is_ok(), "{url}");
        }
        for url in [
            "http://app.example/return",
            "http://localhost.evil.example/",
            "//evil.example",
            "javascript:alert(1)",
            "data:text/html,test",
            "file:///tmp/result",
            "https://user:password@app.example/",
            "https://app.example/#token",
            "https://app.example/\\evil",
            " https://app.example/",
            "https://app.example/\r\nLocation:evil",
        ] {
            let raw = serde_json::json!({"default_url":url}).to_string();
            assert!(
                OAuthReturnRoutes::parse(&raw).is_err(),
                "accepted unsafe URL"
            );
        }
    }

    #[test]
    fn rejects_invalid_config_without_exposing_its_values() {
        for raw in [
            "{}",
            "[]",
            r#"{"default_url":"https://app.example","typo":true}"#,
            r#"{"default_url":"https://app.example","services":{"api-google":{"pages":{"default":"https://app.example"}}}}"#,
        ] {
            assert!(OAuthReturnRoutes::parse(raw).is_err());
        }
        let oversized = "x".repeat(MAX_CONFIG_BYTES + 1);
        assert!(OAuthReturnRoutes::parse(&oversized).is_err());
        let routes = OAuthReturnRoutes::parse(
            r#"{"default_url":"https://app.example/?private_context=example"}"#,
        )
        .unwrap();
        assert!(!format!("{routes:?}").contains("private_context"));
    }

    #[test]
    fn rejects_duplicate_route_names_and_bounded_map_overflow() {
        for raw in [
            r#"{"default_url":"https://app.example","services":{"api-google":{},"api-google":{}}}"#,
            r#"{"default_url":"https://app.example","services":{"api-google":{"pages":{"onboarding":"https://one.example","onboarding":"https://two.example"}}}}"#,
        ] {
            assert!(OAuthReturnRoutes::parse(raw).is_err());
        }
        let pages: BTreeMap<_, _> = (0..=MAX_PAGES)
            .map(|i| (format!("page-{i}"), "https://app.example"))
            .collect();
        let raw = serde_json::json!({"default_url":"https://app.example","services":{"api-google":{"pages":pages}}});
        assert!(OAuthReturnRoutes::parse(&raw.to_string()).is_err());
        let services: BTreeMap<_, _> = (0..=MAX_SERVICES)
            .map(|i| (format!("service-{i}"), serde_json::json!({})))
            .collect();
        let raw = serde_json::json!({"default_url":"https://app.example","services":services});
        assert!(OAuthReturnRoutes::parse(&raw.to_string()).is_err());
        let raw = serde_json::json!({"default_url":format!("https://app.example/{}", "x".repeat(MAX_URL_BYTES))});
        assert!(OAuthReturnRoutes::parse(&raw.to_string()).is_err());
    }
}
