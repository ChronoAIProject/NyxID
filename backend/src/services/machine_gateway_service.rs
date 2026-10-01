//! Metadata-only gateway discovery and fixed git destinations. Execution still
//! resolves live credentials and policy in the ordinary proxy pipeline.
use crate::{
    errors::{AppError, AppResult},
    models::{
        downstream_service::{GitHttp, InferenceWireProtocol, ServiceInference},
        machine_job::DeclaredService,
    },
    services::{catalog_discovery_service, key_service, platform_key_service},
};
use futures::TryStreamExt;
use mongodb::{Database, bson::doc};
use nyxid_machine::gateway::{Environment, GitRewrite, Variable};
use std::collections::{HashMap, HashSet};

#[derive(serde::Deserialize)]
struct CatalogMetadata {
    #[serde(rename = "_id")]
    id: String,
    inference: Option<ServiceInference>,
    git_http: Option<GitHttp>,
    #[serde(flatten)]
    access: platform_key_service::PlatformKeyMetadata,
}

fn catalog_projection() -> mongodb::bson::Document {
    doc! {
        "_id": 1,
        "slug": 1,
        "provider_config_id": 1,
        "inference": 1,
        "git_http": 1,
        "platform_key": 1,
        "is_active": 1,
        "service_type": 1,
        "visibility": 1,
        "service_category": 1,
        "auth_method": 1,
        "requires_user_credential": 1,
        "credential_present": { "$gt": [
            { "$cond": [
                { "$isArray": "$credential_encrypted" },
                { "$size": "$credential_encrypted" },
                { "$binarySize": { "$ifNull": ["$credential_encrypted", mongodb::bson::Binary {
                    subtype: mongodb::bson::spec::BinarySubtype::Generic, bytes: Vec::new(),
                }] } },
            ] }, 0,
        ] },
    }
}

#[derive(Clone)]
pub struct Ingress {
    pub declared_id: String,
    pub git: Option<GitHttp>,
}

impl Ingress {
    /// Only bodies known to use bounded materialization can enter a pool.
    /// Opaque uploads and git packs are one-shot streams, even for an AI pool.
    pub fn buffered(&self, headers: &axum::http::HeaderMap) -> bool {
        self.git.is_none() && structured_body(headers)
    }
}

fn structured_body(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("json") || value.contains("x-www-form-urlencoded"))
}

pub const GIT_MAX_BYTES: usize = 16 * 1024 * 1024 * 1024;

pub struct AvailableService {
    pub user_service: bool,
    pub id: String,
    pub slug: String,
    pub inference: Option<ServiceInference>,
    pub git: Option<GitHttp>,
}

/// Reuse the catalog/MCP instance resolver and the live platform-key ACL.
/// Metadata is read in batches; no credential is materialized for discovery.
pub async fn services(
    db: &Database,
    owner: &str,
    key_id: &str,
) -> AppResult<Vec<AvailableService>> {
    let key = key_service::get_api_key(db, owner, key_id).await?;
    let allowed = key_service::effective_allowed_service_ids(db, &key).await?;
    let grants = platform_key_service::OwnerGrants::load_for_listing(db, owner).await?;
    let mut rows = catalog_discovery_service::agent_services_with_memberships(
        db,
        owner,
        (!key.allow_all_services).then_some(allowed.as_slice()),
        grants.memberships(),
    )
    .await?;
    // The catalog's inventory snapshot includes admin-only org connections
    // visible to members. Machine declarations require execution authority.
    rows.retain(|row| {
        row.user_id == owner
            || grants.memberships().iter().any(|membership| {
                membership.org_user_id == row.user_id
                    && super::user_service_service::role_can_proxy_service(membership.role, row)
            })
    });
    let instance_ids: HashSet<_> = rows.iter().map(|row| row.id.as_str()).collect();
    let catalog_ids: HashSet<_> = rows
        .iter()
        .filter_map(|row| row.catalog_service_id.as_deref())
        .chain(
            allowed
                .iter()
                .map(String::as_str)
                .filter(|id| !instance_ids.contains(id)),
        )
        .collect();
    let catalog: Vec<CatalogMetadata> = db
        .collection(crate::models::downstream_service::COLLECTION_NAME)
        .find(doc! {
            "_id": { "$in": catalog_ids.into_iter().collect::<Vec<_>>() },
            "is_active": true,
            "service_type": { "$ne": "ssh" },
        })
        .projection(catalog_projection())
        .await?
        .try_collect()
        .await?;
    let providers = platform_key_service::load_providers(db).await?;
    let available = |service: &CatalogMetadata| {
        let provider = service
            .access
            .provider_config_id
            .as_ref()
            .and_then(|id| providers.get(id));
        platform_key_service::available_metadata_with_grants(
            &service.access,
            provider,
            owner,
            &grants,
        )
    };
    let metadata_by_id: HashMap<_, _> = catalog.iter().map(|row| (row.id.as_str(), row)).collect();
    let pools = super::service_pool_routing::agent_pools_with_services(
        db,
        owner,
        &rows,
        (!key.allow_all_nodes).then_some(key.allowed_node_ids.as_slice()),
    )
    .await?;
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    let mut rows = rows;
    rows.sort_by_key(|row| (row.user_id != owner, row.slug.clone(), row.id.clone()));
    for row in rows {
        if row.service_type == "ssh" {
            continue;
        }
        let metadata = row
            .catalog_service_id
            .as_ref()
            .and_then(|id| metadata_by_id.get(id.as_str()).copied());
        let platform = platform_key_service::binding(&row) == "platform";
        if platform && !metadata.is_some_and(available) {
            continue;
        }
        if !seen.insert(row.slug.clone()) {
            continue;
        }
        result.push(AvailableService {
            user_service: true,
            id: row.id,
            slug: row.slug,
            inference: metadata.and_then(|entry| entry.inference.clone()),
            git: if platform {
                None
            } else {
                metadata.and_then(|entry| entry.git_http.clone())
            },
        });
    }
    for entry in &catalog {
        if (key.allow_all_services || allowed.contains(&entry.id))
            && available(entry)
            && !seen.contains(&entry.access.slug)
        {
            seen.insert(entry.access.slug.clone());
            result.push(AvailableService {
                user_service: false,
                id: entry.id.clone(),
                slug: entry.access.slug.clone(),
                inference: entry.inference.clone(),
                git: None,
            });
        }
    }
    for pool in pools {
        if seen.contains(&pool.slug) {
            continue;
        }
        // Members have already passed the shared instance ACL and key/node
        // allowlists. Apply the same live platform ACL as ordinary declarations.
        let members: Vec<_> = pool
            .members
            .iter()
            .filter(|member| member.enabled)
            .filter_map(|member| result.iter().find(|row| row.id == member.user_service_id))
            .collect();
        if members.is_empty() {
            continue;
        }
        let inference =
            if pool.member_contract == crate::models::service_pool::PoolMemberContract::AiChat {
                Some(ServiceInference {
                    wire_protocol: InferenceWireProtocol::OpenaiCompletions,
                    model_list: false,
                    realtime: false,
                })
            } else {
                members.first().and_then(|row| row.inference.clone())
            };
        seen.insert(pool.slug.clone());
        result.push(AvailableService {
            user_service: false,
            id: pool.id,
            slug: pool.slug,
            inference,
            git: None,
        });
    }
    Ok(result)
}

pub fn declare(
    requested: &[String],
    available: Vec<AvailableService>,
) -> AppResult<Vec<AvailableService>> {
    if requested.len() > 32 {
        return Err(AppError::ValidationError(
            "Declare at most 32 services per command".into(),
        ));
    }
    let mut selected = Vec::new();
    let mut seen = HashSet::new();
    for selector in requested {
        let mut matches = available
            .iter()
            .filter(|row| &row.id == selector || &row.slug == selector);
        let row = matches.next().ok_or_else(|| AppError::ApiKeyScopeForbidden(
            format!("Service {selector} is not accessible to this agent; connect it or request permission first")
        ))?;
        if matches.next().is_some() {
            return Err(AppError::ValidationError(
                "Ambiguous service; declare its ID".into(),
            ));
        }
        if seen.insert(row.id.clone()) {
            selected.push(AvailableService {
                user_service: row.user_service,
                id: row.id.clone(),
                slug: row.slug.clone(),
                inference: row.inference.clone(),
                git: row.git.clone(),
            });
        }
    }
    Ok(selected)
}

pub fn declared(rows: &[AvailableService]) -> Vec<DeclaredService> {
    rows.iter()
        .map(|row| DeclaredService {
            id: row.id.clone(),
            slug: row.slug.clone(),
        })
        .collect()
}

pub fn environment(rows: &[AvailableService]) -> AppResult<Environment> {
    let mut environment = Environment::default();
    for row in rows {
        if let Some(inference) = &row.inference {
            let (base, key) = match inference.wire_protocol {
                InferenceWireProtocol::AnthropicMessages => {
                    ("ANTHROPIC_BASE_URL", "ANTHROPIC_API_KEY")
                }
                InferenceWireProtocol::OpenaiResponses
                | InferenceWireProtocol::OpenaiCompletions => ("OPENAI_BASE_URL", "OPENAI_API_KEY"),
            };
            environment
                .variables
                .entry(base.into())
                .or_insert_with(|| Variable::GatewayPath(format!("/s/{}", row.slug)));
            environment
                .variables
                .entry(key.into())
                .or_insert(Variable::GatewayToken);
        }
        if let Some(git) = &row.git {
            let origin = git_origin(git)?;
            environment.git.push(GitRewrite {
                origin: git.origin.clone(),
                path: format!(
                    "/git/{}/",
                    &origin[url::Position::BeforeHost..url::Position::AfterPort]
                ),
            });
        }
    }
    Ok(environment)
}

pub fn git_host(git: &GitHttp) -> AppResult<String> {
    let origin = git_origin(git)?;
    Ok(origin[url::Position::BeforeHost..url::Position::AfterPort].to_owned())
}

fn git_origin(git: &GitHttp) -> AppResult<url::Url> {
    let url = url::Url::parse(&git.origin)
        .map_err(|_| AppError::ValidationError("Invalid catalog git origin".into()))?;
    if url.scheme() != "https"
        || url.origin().ascii_serialization() != git.origin
        || url.host_str().is_none()
        || git.username.is_empty()
        || git.username.contains(':')
    {
        return Err(AppError::ValidationError(
            "Git requires an exact HTTPS catalog origin".into(),
        ));
    }
    Ok(url)
}

pub fn git_path<'a>(raw: &'a str, method: &str) -> AppResult<(&'a str, &'a str)> {
    let (host, path) = raw
        .strip_prefix("/git/")
        .and_then(|p| p.split_once('/'))
        .ok_or_else(|| AppError::ValidationError("Use /git/{host}/{repository}".into()))?;
    let (resource, query) = path.split_once('?').unwrap_or((path, ""));
    let parts: Vec<_> = resource.split('/').collect();
    let valid_name = |s: &str| {
        !s.is_empty()
            && s != "."
            && s != ".."
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    if parts.len() < 3 || !valid_name(parts[0]) || !valid_name(parts[1]) {
        return Err(AppError::ValidationError(
            "Invalid git repository path".into(),
        ));
    }
    let route = parts[2..].join("/");
    let valid = match (method, route.as_str()) {
        ("GET", "info/refs") => matches!(
            query,
            "service=git-upload-pack" | "service=git-receive-pack"
        ),
        ("POST", "git-upload-pack" | "git-receive-pack") => query.is_empty(),
        _ => false,
    };
    if !valid {
        return Err(AppError::ValidationError(
            "Only git smart-HTTP discovery, fetch and push are supported".into(),
        ));
    }
    Ok((host, path))
}

pub fn apply_git_target(
    target: &mut super::proxy_service::ProxyTarget,
    master: bool,
    requested: &GitHttp,
) -> AppResult<()> {
    if master
        || target.service.git_http.as_ref() != Some(requested)
        || !target.service.destination_targets.is_empty()
    {
        return Err(AppError::Forbidden(
            "Git requires the owner's connected credential and live catalog git metadata; platform keys are not used".into(),
        ));
    }
    git_origin(requested)?;
    target.base_url = requested.origin.clone();
    target.auth_method = "github_git".into();
    target.auth_key_name = requested.username.clone();
    Ok(())
}

/// Keep structured adapters on their existing bounded inspection path. Git and
/// opaque HTTP uploads need no body interpretation for policy or credentials.
pub fn can_stream(
    target: &super::proxy_service::ProxyTarget,
    headers: &axum::http::HeaderMap,
    git: bool,
) -> bool {
    if git {
        return true;
    }
    !structured_body(headers)
        && matches!(
            target.auth_method.as_str(),
            "none"
                | "bearer"
                | "header"
                | "basic"
                | "bot_bearer"
                | "query"
                | "path"
                | "token_exchange"
        )
        && target.service.inference.is_none()
}

pub struct UploadMeter {
    total: std::sync::atomic::AtomicU64,
    over: std::sync::atomic::AtomicBool,
    pub limit: usize,
}

impl UploadMeter {
    pub fn bytes(&self) -> i64 {
        self.total
            .load(std::sync::atomic::Ordering::Relaxed)
            .min(i64::MAX as u64) as i64
    }
    pub fn exceeded(&self) -> bool {
        self.over.load(std::sync::atomic::Ordering::Relaxed)
    }
    pub fn error(&self) -> AppError {
        AppError::RequestBodyTooLarge {
            max_bytes: self.limit,
            context: "Machine gateway".into(),
        }
    }
}

pub fn stream_upload(
    request: axum::http::Request<axum::body::Body>,
    limit: usize,
) -> AppResult<(axum::body::Body, std::sync::Arc<UploadMeter>)> {
    use futures::StreamExt;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    };
    let meter = Arc::new(UploadMeter {
        total: AtomicU64::new(0),
        over: AtomicBool::new(false),
        limit,
    });
    if request
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|n| n > limit as u64)
    {
        return Err(meter.error());
    }
    let progress = meter.clone();
    let stream = request.into_body().into_data_stream().map(move |chunk| {
        let chunk = chunk.map_err(|_| std::io::Error::other("machine upload interrupted"))?;
        let total = progress
            .total
            .fetch_add(chunk.len() as u64, Ordering::Relaxed)
            .saturating_add(chunk.len() as u64);
        if total > limit as u64 {
            progress.over.store(true, Ordering::Relaxed);
            return Err(std::io::Error::other("machine upload limit exceeded"));
        }
        Ok(chunk)
    });
    Ok((axum::body::Body::from_stream(stream), meter))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn opaque_uploads_are_lazy_and_enforce_limits_without_content_length() {
        use axum::{body::Body, http::Request};
        use futures::StreamExt;
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let polls = Arc::new(AtomicUsize::new(0));
        let observed = polls.clone();
        let input = futures::stream::iter([
            bytes::Bytes::from_static(b"first"),
            bytes::Bytes::from_static(b"second"),
        ])
        .map(move |bytes| {
            observed.fetch_add(1, Ordering::Relaxed);
            Ok::<_, std::io::Error>(bytes)
        });
        let (body, meter) = stream_upload(Request::new(Body::from_stream(input)), 6).unwrap();
        assert_eq!(polls.load(Ordering::Relaxed), 0);
        let mut stream = body.into_data_stream();
        assert_eq!(stream.next().await.unwrap().unwrap(), "first");
        assert_eq!(polls.load(Ordering::Relaxed), 1);
        assert!(stream.next().await.unwrap().is_err());
        assert!(meter.exceeded());
        assert!(matches!(
            meter.error(),
            AppError::RequestBodyTooLarge { max_bytes: 6, .. }
        ));
        assert!(
            stream_upload(
                Request::builder()
                    .header("content-length", 7)
                    .body(Body::empty())
                    .unwrap(),
                6
            )
            .is_err()
        );
    }
    #[test]
    fn declarations_and_environment_follow_catalog_metadata_not_slugs() {
        let available = vec![AvailableService {
            user_service: true,
            id: "service-id".into(),
            slug: "custom-company-model".into(),
            inference: Some(ServiceInference {
                wire_protocol: InferenceWireProtocol::AnthropicMessages,
                model_list: false,
                realtime: false,
            }),
            git: Some(GitHttp {
                origin: "https://git.example.test:8443".into(),
                username: "oauth2".into(),
            }),
        }];
        assert!(declare(&["ungranted".into()], Vec::new()).is_err());
        let selected = declare(
            &["service-id".into(), "custom-company-model".into()],
            available,
        )
        .unwrap();
        assert_eq!(selected.len(), 1);
        let env = environment(&selected).unwrap();
        assert_eq!(
            env.variables["ANTHROPIC_BASE_URL"],
            Variable::GatewayPath("/s/custom-company-model".into())
        );
        assert_eq!(env.variables["ANTHROPIC_API_KEY"], Variable::GatewayToken);
        assert!(!env.variables.contains_key("OPENAI_API_KEY"));
        assert_eq!(env.git[0].origin, "https://git.example.test:8443");
        assert_eq!(env.git[0].path, "/git/git.example.test:8443/");
        let empty = environment(&[]).unwrap();
        assert!(empty.variables.is_empty() && empty.git.is_empty());
    }

    #[test]
    fn only_fixed_smart_http_routes_are_admitted() {
        assert!(
            git_path(
                "/git/github/owner/repo.git/info/refs?service=git-upload-pack",
                "GET"
            )
            .is_ok()
        );
        assert!(git_path("/git/github/owner/repo.git/git-receive-pack", "POST").is_ok());
        for p in [
            "/git/github/../repo.git/git-upload-pack",
            "/git/github/%2fexample/repo/git-upload-pack",
            "/git/github/owner/repo.git/info/refs?service=git-upload-pack&token=bad",
            "/git/gitlab/owner/repo/git-upload-pack",
            "/git/github/owner/repo.git/config",
        ] {
            assert!(git_path(p, "GET").is_err());
        }
    }
}
