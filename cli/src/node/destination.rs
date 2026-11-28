use serde::{Deserialize, Serialize};
use url::Url;

use super::error::{Error, Result};

/// Local authorization is independent of the mutable routing hint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestinationBinding {
    version: u8,
    destinations: Vec<String>,
}

fn invalid_destination() -> Error {
    Error::Validation("Expected an absolute destination URL with an optional path prefix".into())
}

fn parse_base(raw: &str) -> Result<Url> {
    let url = Url::parse(raw).map_err(|_| invalid_destination())?;
    if !matches!(url.scheme(), "http" | "https" | "ws" | "wss")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || raw.contains('\\')
        || raw.bytes().any(|b| b.is_ascii_control())
    {
        return Err(invalid_destination());
    }
    Ok(url)
}

impl DestinationBinding {
    pub fn new<'a>(urls: impl IntoIterator<Item = &'a str>) -> Result<Self> {
        let mut destinations = Vec::new();
        for raw in urls {
            let url = parse_base(raw)?;
            if !safe_scoped_path(url.path()) {
                return Err(invalid_destination());
            }
            destinations.push(url.to_string());
        }
        destinations.sort();
        destinations.dedup();
        Ok(Self {
            version: 1,
            destinations,
        })
    }

    pub fn ssh_endpoint(raw: &str) -> Result<(String, u16)> {
        let url = Url::parse(raw).map_err(|_| invalid_destination())?;
        if url.scheme() != "ssh"
            || !url.username().is_empty()
            || url.password().is_some()
            || !matches!(url.path(), "" | "/")
            || url.query().is_some()
            || url.fragment().is_some()
            || raw.bytes().any(|b| b.is_ascii_control())
        {
            return Err(invalid_destination());
        }
        let host = match url.host().ok_or_else(invalid_destination)? {
            url::Host::Ipv6(ip) => ip.to_string(),
            h => h.to_string(),
        };
        let port = url.port().unwrap_or(22);
        if port == 0 {
            return Err(invalid_destination());
        }
        Ok((host, port))
    }

    pub fn description(&self) -> String {
        if self.version != 1 || self.destinations.is_empty() {
            return "no approved destination".into();
        }
        self.destinations.join(", ")
    }

    pub fn permits(&self, base: &str, path: &str, ws: bool) -> bool {
        let Ok(base_url) = parse_base(base) else {
            return false;
        };
        if self.version != 1
            || path.contains(['\\', '#'])
            || path.bytes().any(|b| b.is_ascii_control())
        {
            return false;
        }
        // The executor appends a path to the base; an absolute-path join would discard its prefix.
        let endpoint = format!(
            "{}/{}",
            base.trim_end_matches('/'),
            path.strip_prefix('/').unwrap_or(path)
        );
        let Ok(actual) = Url::parse(&endpoint) else {
            return false;
        };
        if actual.origin() != base_url.origin() {
            return false;
        }
        self.permits_url(actual, ws)
    }

    pub fn permits_url(&self, mut actual: Url, ws: bool) -> bool {
        if self.version != 1
            || !actual.username().is_empty()
            || actual.password().is_some()
            || actual.fragment().is_some()
        {
            return false;
        }
        if ws {
            transport_scheme(&mut actual);
        }
        self.destinations.iter().any(|raw| {
            let Ok(mut allowed) = parse_base(raw) else {
                return false;
            };
            if ws {
                transport_scheme(&mut allowed);
            }
            let prefix = allowed.path().trim_end_matches('/');
            allowed.origin() == actual.origin()
                && (prefix.is_empty()
                    || (safe_scoped_path(actual.path())
                        && (actual.path() == prefix
                            || actual
                                .path()
                                .strip_prefix(prefix)
                                .is_some_and(|tail| tail.starts_with('/')))))
        })
    }
}

// HTTP(S) endpoints use the corresponding WS(S) handshake transport.
fn transport_scheme(url: &mut Url) {
    match url.scheme() {
        "https" => {
            let _ = url.set_scheme("wss");
        }
        "http" => {
            let _ = url.set_scheme("ws");
        }
        _ => {}
    }
}

fn safe_scoped_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    !["%2f", "%5c", "%25", "%2e", "%00"]
        .iter()
        .any(|escaped| lower.contains(escaped))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_compares_transport_host_effective_port_and_path_segments() {
        let binding = DestinationBinding::new(["https://example.test:443/api"]).unwrap();
        for (base, path, ws, allowed) in [
            ("https://example.test", "/api", false, true),
            ("https://example.test/api", "messages", false, true),
            ("wss://example.test/api", "/socket", true, true),
            ("http://example.test/api", "/", false, false),
            ("https://example.test:444/api", "/", false, false),
            ("https://another.test/api", "/", false, false),
            ("https://example.test", "/apis", false, false),
            ("https://example.test/api", "../other", false, false),
            ("https://example.test/api", "%2Foutside", false, false),
            ("ws://example.test/api", "/", true, false),
        ] {
            assert_eq!(binding.permits(base, path, ws), allowed);
        }
    }

    #[test]
    fn invalid_binding_versions_and_empty_destination_sets_refuse_requests() {
        let unknown: DestinationBinding = serde_json::from_value(serde_json::json!({
            "version": 2, "destinations": ["https://example.test"]
        }))
        .unwrap();
        for binding in [unknown, DestinationBinding::new([]).unwrap()] {
            assert!(!binding.permits("https://example.test", "/", false));
        }
    }
}
