//! Typed provider resources from authorized reads. Never scan arbitrary JSON for URLs.
use super::{mcp_service as mcp, proxy_service::ProxyTarget};
use crate::models::assistant_resource::ResourceSource;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const MAX_SOURCE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_RESOURCES: usize = 32;

/// Keep the application's connection bounds when disabling redirects for a source read.
pub(crate) fn http_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
        let builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(10))
            .pool_idle_timeout(std::time::Duration::from_secs(90));
        #[cfg(test)]
        let builder = builder.pool_max_idle_per_host(0);
        builder.build().expect("source HTTP client")
    });
    &CLIENT
}

pub struct ResourceCandidate {
    pub selector: String,
    pub revision: String,
    pub label: String,
    url: String,
}
impl std::fmt::Debug for ResourceCandidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceCandidate").finish_non_exhaustive()
    }
}
impl ResourceCandidate {
    pub(crate) fn url(&self) -> &str {
        &self.url
    }
}

pub struct ResourceRead {
    pub source: ResourceSource,
    pub arguments: Value,
    pub candidates: Vec<ResourceCandidate>,
}
impl std::fmt::Debug for ResourceRead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceRead")
            .field("count", &self.candidates.len())
            .finish_non_exhaustive()
    }
}

fn uuid(input: &str) -> Option<String> {
    uuid::Uuid::parse_str(input).ok().map(|id| id.to_string())
}

/// Only the two published read operations are replayable. Arguments are reconstructed
/// from the validated execution, never copied wholesale from a tool request.
pub fn read_arguments(template: &str, path: &str, query: Option<&str>) -> Option<Value> {
    let (prefix, suffix, key) = match template {
        "/v1/pages/{page_id}" => ("v1/pages/", "", "page_id"),
        "/v1/blocks/{block_id}/children" => ("v1/blocks/", "/children", "block_id"),
        _ => return None,
    };
    // MCP's request builder removes the leading slash; canonical forwarding
    // can restore it. Accept either form, without collapsing malformed slashes.
    let path = path.strip_prefix('/').unwrap_or(path);
    let id = uuid(path.strip_prefix(prefix)?.strip_suffix(suffix)?)?;
    let mut args = json!({key: id});
    for (key, value) in url::form_urlencoded::parse(query.unwrap_or_default().as_bytes()) {
        let key: &str = &key;
        if args.get(key).is_some() {
            return None;
        }
        match key {
            "start_cursor"
                if template.contains("blocks") && value.len() <= 200 && !value.is_empty() =>
            {
                args[key] = value.into_owned().into();
            }
            "page_size" if template.contains("blocks") => {
                let count: u32 = value.parse().ok()?;
                if !(1..=100).contains(&count) {
                    return None;
                }
                args[key] = count.into();
            }
            _ => return None,
        }
    }
    Some(args)
}

pub fn source(
    service: &mcp::McpToolService,
    endpoint: &mcp::McpToolEndpoint,
    target: &ProxyTarget,
    node_routed: bool,
    catalog_slug: Option<&str>,
) -> Option<ResourceSource> {
    if node_routed
        || !matches!(service.source, mcp::McpToolSource::UserManaged { .. })
        || target.auth_method != "bearer"
        || target.service.identity_propagation_mode != "none"
        || target.service.forward_access_token
        || target.service.inject_delegation_token
        || service.is_generic_proxy
        || endpoint.method != "GET"
        || !matches!(
            endpoint.path.as_str(),
            "/v1/pages/{page_id}" | "/v1/blocks/{block_id}/children"
        )
        || catalog_slug != Some("api-notion")
        || !trusted_notion_base(&target.base_url)
    {
        return None;
    }
    let generation = mcp::producer_operation_generation(service, endpoint)?;
    // A changed account, credential, destination or header configuration requires a
    // new source read. Automatic OAuth refresh can conservatively invalidate a handle.
    // Only the digest persists; neither credentials nor header values are returned.
    let authority_digest = mcp::canonical_sha256(json!({
        "v":1, "catalog_id":target.service.id, "target":target.target_id,
        "base":target.base_url, "auth":target.auth_method, "key":target.auth_key_name,
        "scheme":target.service.auth_scheme,
        "credential":hex::encode(Sha256::digest(target.credential.as_bytes())),
        "catalog_headers":target.catalog_default_headers, "headers":target.user_service_default_headers,
        "connection":target.connection_id,
    }));
    Some(ResourceSource {
        adapter: "notion.v1".into(),
        service_id: service.service_id.clone(),
        endpoint_id: endpoint.endpoint_id.clone(),
        generation,
        endpoint_digest: mcp::endpoint_contract_digest(endpoint),
        authority_digest,
        tool_name: format!("{}__{}", service.service_slug, endpoint.name),
    })
}

fn trusted_notion_base(base: &str) -> bool {
    #[cfg(test)]
    if TEST_ORIGINS.contains_key(base) {
        return true;
    }
    url::Url::parse(base).is_ok_and(|u| {
        u.scheme() == "https"
            && u.host_str() == Some("api.notion.com")
            && u.port_or_known_default() == Some(443)
            && u.username().is_empty()
            && u.password().is_none()
            && matches!(u.path(), "" | "/")
            && u.query().is_none()
            && u.fragment().is_none()
    })
}

fn revision(object: &Value) -> Option<&str> {
    let value = object.get("last_edited_time")?.as_str()?;
    chrono::DateTime::parse_from_rfc3339(value).ok()?;
    (value.len() <= 64).then_some(value)
}

fn add_file(
    out: &mut Vec<ResourceCandidate>,
    file: &Value,
    selector: String,
    rev: &str,
    label: &str,
) {
    if out.len() >= MAX_RESOURCES {
        return;
    }
    let kind = file.get("type").and_then(Value::as_str);
    let url = match kind {
        Some("file") => file.pointer("/file/url"),
        Some("external") => file.pointer("/external/url"),
        _ => None,
    }
    .and_then(Value::as_str);
    let Some(url) = url.filter(|s| s.len() <= super::channel_media_service::MAX_MEDIA_URL_BYTES)
    else {
        return;
    };
    // The downloader additionally validates public DNS, pins addresses and refuses redirects.
    if !url::Url::parse(url).is_ok_and(|u| {
        let allowed_scheme = u.scheme() == "https";
        #[cfg(test)]
        let allowed_scheme = allowed_scheme || test_media_origin(url).is_some();
        allowed_scheme
            && u.username().is_empty()
            && u.password().is_none()
            && u.fragment().is_none()
    }) {
        return;
    }
    out.push(ResourceCandidate {
        selector,
        revision: rev.into(),
        label: super::assistant_upload_service::safe_name(label),
        url: url.into(),
    });
}

pub fn candidates(arguments: &Value, text: &str) -> Vec<ResourceCandidate> {
    if text.len() > MAX_SOURCE_BYTES {
        return Vec::new();
    }
    let Ok(root) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(page_id) = arguments.get("page_id").and_then(Value::as_str) {
        if root["object"] != "page"
            || root["id"].as_str().and_then(uuid).as_deref() != Some(page_id)
        {
            return out;
        }
        let Some(rev) = revision(&root) else {
            return out;
        };
        for field in ["cover", "icon"] {
            add_file(
                &mut out,
                &root[field],
                format!("page:{page_id}:{field}"),
                rev,
                field,
            );
        }
        if let Some(properties) = root["properties"].as_object() {
            for property in properties
                .values()
                .filter(|p| p["type"] == "files")
                .take(100)
            {
                if out.len() >= MAX_RESOURCES {
                    break;
                }
                let Some(property_id) = property["id"].as_str().filter(|id| id.len() <= 200) else {
                    continue;
                };
                let Some(files) = property["files"].as_array() else {
                    continue;
                };
                if files.len() > 100 {
                    continue;
                }
                for file in files {
                    let Some(name) = file["name"].as_str().filter(|s| s.len() <= 200) else {
                        continue;
                    };
                    if files
                        .iter()
                        .filter(|f| f["name"].as_str() == Some(name))
                        .count()
                        != 1
                    {
                        continue;
                    }
                    let selector = format!(
                        "page:{page_id}:file:{}",
                        mcp::canonical_sha256(json!([property_id, name]))
                    );
                    add_file(&mut out, file, selector, rev, name);
                }
            }
        }
    } else if arguments.get("block_id").is_some()
        && root["object"] == "list"
        && let Some(blocks) = root["results"].as_array()
    {
        if blocks.len() > 100 {
            return out;
        }
        for block in blocks.iter().take(100) {
            if block["object"] != "block" {
                continue;
            }
            let Some(id) = block["id"].as_str().and_then(uuid) else {
                continue;
            };
            // Duplicate object IDs are ambiguous and must not create a selectable resource.
            if blocks
                .iter()
                .filter(|b| b["id"].as_str().and_then(uuid).as_ref() == Some(&id))
                .count()
                != 1
            {
                continue;
            }
            let Some(rev) = revision(block) else {
                continue;
            };
            if block["type"] == "image" {
                add_file(
                    &mut out,
                    &block["image"],
                    format!("block:{id}:image"),
                    rev,
                    "Notion image",
                );
            }
        }
    }
    out
}

// Exact mock origins belong to the test that owns the listener. These
// exceptions and their registry are absent from production binaries.
#[cfg(test)]
static TEST_ORIGINS: std::sync::LazyLock<dashmap::DashMap<String, ()>> =
    std::sync::LazyLock::new(dashmap::DashMap::new);
#[cfg(test)]
pub(crate) struct TestOrigin(String);
#[cfg(test)]
impl Drop for TestOrigin {
    fn drop(&mut self) {
        TEST_ORIGINS.remove(&self.0);
    }
}
#[cfg(test)]
pub(crate) fn register_test_origin(base: &str) -> TestOrigin {
    let parsed = url::Url::parse(base).unwrap();
    assert_eq!(parsed.scheme(), "http");
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    assert!(parsed.port().is_some());
    assert_eq!(parsed.path(), "/");
    TEST_ORIGINS.insert(base.into(), ());
    TestOrigin(base.into())
}
#[cfg(test)]
pub(crate) fn test_media_origin(url: &str) -> Option<String> {
    let origin = url::Url::parse(url).ok()?.origin().ascii_serialization();
    TEST_ORIGINS.contains_key(&origin).then_some(origin)
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "fe63677d-5d00-4da4-8a43-908c0175b440";

    #[tokio::test]
    async fn resource_source_client_bounds_a_stalled_tls_connection() {
        // The kernel accepts TCP while no server answers the TLS handshake.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let request =
            http_client().get(format!("https://{}/source", listener.local_addr().unwrap()));
        let error = tokio::time::timeout(std::time::Duration::from_secs(15), request.send())
            .await
            .expect("source connect timeout must bound the handshake")
            .unwrap_err();
        assert!(error.is_timeout() && error.is_connect(), "{error}");
    }

    #[test]
    fn only_canonical_source_and_allowlisted_reads() {
        assert!(trusted_notion_base("https://api.notion.com"));
        for base in [
            "https://api.notion.com.evil.test",
            "https://api.notion.com/v1",
            "http://api.notion.com",
            "https://x@api.notion.com",
            "https://api.notion.com?x=1",
        ] {
            assert!(!trusted_notion_base(base));
        }
        let path = format!("/v1/blocks/{ID}/children");
        assert_eq!(
            read_arguments(
                "/v1/blocks/{block_id}/children",
                &path,
                Some("page_size=12&start_cursor=a")
            ),
            Some(json!({"block_id":ID,"page_size":12,"start_cursor":"a"}))
        );
        assert_eq!(
            read_arguments("/v1/blocks/{block_id}/children", &path[1..], None),
            Some(json!({"block_id":ID}))
        );
        for path in [format!("v1/pages/{ID}"), format!("/v1/pages/{ID}")] {
            assert_eq!(
                read_arguments("/v1/pages/{page_id}", &path, None),
                Some(json!({"page_id":ID}))
            );
        }
        assert!(
            read_arguments("/v1/blocks/{block_id}/children", &format!("/{path}"), None).is_none()
        );
        for q in [
            "url=https://example.org",
            "page_size=101",
            "page_size=1&page_size=2",
            "headers=x",
        ] {
            assert!(read_arguments("/v1/blocks/{block_id}/children", &path, Some(q)).is_none());
        }
        assert!(read_arguments("/v1/pages/{page_id}", "/v1/pages/../../x", None).is_none());
    }
    #[test]
    fn stable_object_selectors_ignore_array_order_and_bind_revision() {
        let image = json!({"object":"block","id":ID,"type":"image","last_edited_time":"2026-10-17T00:00:00Z","image":{"type":"file","file":{"url":"https://files.example.org/image.png?secret=one"}}});
        let other = json!({"object":"block","id":"7cd20efb-cfa5-4092-86de-88c035190aec","type":"paragraph"});
        let args = json!({"block_id":ID});
        let a = candidates(
            &args,
            &json!({"object":"list","results":[image.clone(),other.clone()]}).to_string(),
        );
        let b = candidates(
            &args,
            &json!({"object":"list","results":[other,image.clone()]}).to_string(),
        );
        assert_eq!(a[0].selector, b[0].selector);
        assert_eq!(a[0].revision, "2026-10-17T00:00:00Z");
        assert!(!format!("{:?}", a[0]).contains("secret"));
        assert!(
            candidates(
                &args,
                &json!({"object":"list","results":[image.clone(),image]}).to_string()
            )
            .is_empty()
        );
        assert!(
            candidates(
                &args,
                &json!({"arbitrary":{"url":"https://example.org/a.png"}}).to_string()
            )
            .is_empty()
        );
    }
    #[test]
    fn page_files_require_unique_names_and_source_revision() {
        let file = json!({"name":"photo.png","type":"external","external":{"url":"https://example.org/a.png"}});
        let mut page = json!({"object":"page","id":ID,"last_edited_time":"2026-10-17T00:00:00Z","properties":{"Pictures":{"id":"abc","type":"files","files":[file.clone()]}}});
        let args = json!({"page_id":ID});
        assert_eq!(candidates(&args, &page.to_string()).len(), 1);
        page["properties"]["Pictures"]["files"] = json!([file.clone(), file]);
        assert!(candidates(&args, &page.to_string()).is_empty());
        page["last_edited_time"] = Value::Null;
        assert!(candidates(&args, &page.to_string()).is_empty());
    }
}
