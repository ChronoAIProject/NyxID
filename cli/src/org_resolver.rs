use std::fmt;
use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use serde::Deserialize;
use uuid::Uuid;

use crate::api::ApiClient;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrgCandidate {
    pub id: String,
    pub slug: String,
    pub display_name: Option<String>,
}

pub struct OrgResolver<'a> {
    api: &'a mut ApiClient,
    cached_orgs: Option<Vec<OrgCandidate>>,
}

#[derive(Debug)]
pub enum OrgResolveError {
    NotFound(String),
    Ambiguous {
        input: String,
        candidates: Vec<OrgCandidate>,
    },
    Api(anyhow::Error),
}

impl fmt::Display for OrgResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(input) => write!(
                f,
                "Organization not found for '{input}'. Pass an org UUID, slug, or exact display name."
            ),
            Self::Ambiguous { input, candidates } => {
                writeln!(f, "Organization display name is ambiguous for '{input}'.")?;
                writeln!(f, "Use a unique slug or UUID:")?;
                writeln!(f, "SLUG  ID  DISPLAY NAME")?;
                for candidate in candidates {
                    writeln!(
                        f,
                        "{}  {}  {}",
                        candidate.slug,
                        candidate.id,
                        candidate.display_name.as_deref().unwrap_or("-")
                    )?;
                }
                Ok(())
            }
            Self::Api(err) => write!(f, "Failed to resolve organization: {err}"),
        }
    }
}

impl std::error::Error for OrgResolveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Api(err) => Some(err.as_ref()),
            _ => None,
        }
    }
}

impl<'a> OrgResolver<'a> {
    pub fn new(api: &'a mut ApiClient) -> Self {
        Self {
            api,
            cached_orgs: None,
        }
    }

    pub async fn resolve(&mut self, input: &str) -> Result<String, OrgResolveError> {
        let mut lookup = ApiOrgLookup { api: self.api };
        resolve_with_lookup(&mut lookup, &mut self.cached_orgs, input).await
    }
}

pub async fn resolve_org_id(api: &mut ApiClient, input: &str) -> Result<String> {
    let mut resolver = OrgResolver::new(api);
    resolver.resolve(input).await.map_err(anyhow::Error::new)
}

#[derive(Debug, Deserialize)]
struct OrgResponse {
    id: String,
    slug: String,
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OrgListResponse {
    orgs: Vec<OrgResponse>,
}

impl From<OrgResponse> for OrgCandidate {
    fn from(value: OrgResponse) -> Self {
        Self {
            id: value.id,
            slug: value.slug,
            display_name: value.display_name,
        }
    }
}

trait OrgLookup {
    fn get_org_by_key<'a>(
        &'a mut self,
        key: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<OrgCandidate>>> + 'a>>;

    fn list_orgs<'a>(&'a mut self)
    -> Pin<Box<dyn Future<Output = Result<Vec<OrgCandidate>>> + 'a>>;
}

struct ApiOrgLookup<'a> {
    api: &'a mut ApiClient,
}

impl OrgLookup for ApiOrgLookup<'_> {
    fn get_org_by_key<'a>(
        &'a mut self,
        key: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<OrgCandidate>>> + 'a>> {
        Box::pin(async move {
            let org: Option<OrgResponse> = self.api.get_optional(&format!("/orgs/{key}")).await?;
            Ok(org.map(Into::into))
        })
    }

    fn list_orgs<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<OrgCandidate>>> + 'a>> {
        Box::pin(async move {
            let response: OrgListResponse = self.api.get("/orgs").await?;
            Ok(response.orgs.into_iter().map(Into::into).collect())
        })
    }
}

async fn resolve_with_lookup<L: OrgLookup>(
    lookup: &mut L,
    cached_orgs: &mut Option<Vec<OrgCandidate>>,
    input: &str,
) -> Result<String, OrgResolveError> {
    if Uuid::parse_str(input).is_ok() {
        return Ok(input.to_string());
    }

    if is_slug_shape(input)
        && let Some(org) = lookup
            .get_org_by_key(input)
            .await
            .map_err(OrgResolveError::Api)?
    {
        return Ok(org.id);
    }

    if cached_orgs.is_none() {
        *cached_orgs = Some(lookup.list_orgs().await.map_err(OrgResolveError::Api)?);
    }

    let matches: Vec<OrgCandidate> = cached_orgs
        .as_ref()
        .expect("org cache populated")
        .iter()
        .filter(|org| {
            org.display_name
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(input))
        })
        .cloned()
        .collect();

    match matches.len() {
        0 => Err(OrgResolveError::NotFound(input.to_string())),
        1 => Ok(matches[0].id.clone()),
        _ => Err(OrgResolveError::Ambiguous {
            input: input.to_string(),
            candidates: matches,
        }),
    }
}

fn is_slug_shape(input: &str) -> bool {
    let bytes = input.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return false;
    }
    if !bytes[0].is_ascii_lowercase() && !bytes[0].is_ascii_digit() {
        return false;
    }
    bytes[1..]
        .iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
        && bytes.iter().any(|b| b.is_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct FakeOrgLookup {
        by_key: HashMap<String, Option<OrgCandidate>>,
        orgs: Vec<OrgCandidate>,
        key_calls: usize,
        list_calls: usize,
    }

    impl OrgLookup for FakeOrgLookup {
        fn get_org_by_key<'a>(
            &'a mut self,
            key: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<Option<OrgCandidate>>> + 'a>> {
            Box::pin(async move {
                self.key_calls += 1;
                Ok(self.by_key.get(key).cloned().unwrap_or(None))
            })
        }

        fn list_orgs<'a>(
            &'a mut self,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<OrgCandidate>>> + 'a>> {
            Box::pin(async move {
                self.list_calls += 1;
                Ok(self.orgs.clone())
            })
        }
    }

    fn candidate(id: &str, slug: &str, display_name: &str) -> OrgCandidate {
        OrgCandidate {
            id: id.to_string(),
            slug: slug.to_string(),
            display_name: Some(display_name.to_string()),
        }
    }

    async fn resolve_fake(
        lookup: &mut FakeOrgLookup,
        cache: &mut Option<Vec<OrgCandidate>>,
        input: &str,
    ) -> Result<String, OrgResolveError> {
        resolve_with_lookup(lookup, cache, input).await
    }

    #[tokio::test]
    async fn uuid_input_returns_without_roundtrip() {
        let mut lookup = FakeOrgLookup::default();
        let mut cache = None;
        let input = "550e8400-e29b-41d4-a716-446655440000";

        let resolved = resolve_fake(&mut lookup, &mut cache, input)
            .await
            .expect("resolve uuid");

        assert_eq!(resolved, input);
        assert_eq!(lookup.key_calls, 0);
        assert_eq!(lookup.list_calls, 0);
    }

    #[tokio::test]
    async fn slug_hit_returns_org_id() {
        let mut lookup = FakeOrgLookup::default();
        lookup.by_key.insert(
            "chrono-ai".to_string(),
            Some(candidate("org-1", "chrono-ai", "Chrono AI")),
        );
        let mut cache = None;

        let resolved = resolve_fake(&mut lookup, &mut cache, "chrono-ai")
            .await
            .expect("resolve slug");

        assert_eq!(resolved, "org-1");
        assert_eq!(lookup.key_calls, 1);
        assert_eq!(lookup.list_calls, 0);
    }

    #[test]
    fn slug_shape_rejects_all_digits() {
        assert!(!is_slug_shape("12345"));
    }

    #[test]
    fn slug_shape_accepts_mixed_alphanumeric() {
        assert!(is_slug_shape("abc-123"));
    }

    #[tokio::test]
    async fn display_name_hit_returns_org_id() {
        let mut lookup = FakeOrgLookup {
            orgs: vec![candidate("org-1", "chrono-ai", "Chrono AI")],
            ..Default::default()
        };
        let mut cache = None;

        let resolved = resolve_fake(&mut lookup, &mut cache, "chrono ai")
            .await
            .expect("resolve display name");

        assert_eq!(resolved, "org-1");
        assert_eq!(lookup.key_calls, 0);
        assert_eq!(lookup.list_calls, 1);
    }

    #[tokio::test]
    async fn ambiguous_display_name_returns_candidates() {
        let mut lookup = FakeOrgLookup {
            orgs: vec![
                candidate("org-1", "acme", "Acme"),
                candidate("org-2", "acme-research", "ACME"),
            ],
            ..Default::default()
        };
        let mut cache = None;

        let err = resolve_fake(&mut lookup, &mut cache, "acme")
            .await
            .expect_err("display name should be ambiguous");

        match err {
            OrgResolveError::Ambiguous { input, candidates } => {
                assert_eq!(input, "acme");
                assert_eq!(candidates.len(), 2);
                assert!(candidates.iter().any(|org| org.slug == "acme"));
                assert!(candidates.iter().any(|org| org.slug == "acme-research"));
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[tokio::test]
    async fn display_name_miss_returns_not_found() {
        let mut lookup = FakeOrgLookup {
            orgs: vec![candidate("org-1", "chrono-ai", "Chrono AI")],
            ..Default::default()
        };
        let mut cache = None;

        let err = resolve_fake(&mut lookup, &mut cache, "Missing Org")
            .await
            .expect_err("display name should miss");

        assert!(matches!(err, OrgResolveError::NotFound(input) if input == "Missing Org"));
    }

    #[tokio::test]
    async fn slug_miss_falls_back_to_display_name() {
        let mut lookup = FakeOrgLookup {
            orgs: vec![candidate("org-1", "chrono-ai", "chrono-ai")],
            ..Default::default()
        };
        lookup.by_key.insert("chrono-ai".to_string(), None);
        let mut cache = None;

        let resolved = resolve_fake(&mut lookup, &mut cache, "chrono-ai")
            .await
            .expect("slug miss falls back to name");

        assert_eq!(resolved, "org-1");
        assert_eq!(lookup.key_calls, 1);
        assert_eq!(lookup.list_calls, 1);
    }

    #[test]
    fn is_slug_shape_rejects_empty() {
        assert!(!is_slug_shape(""));
    }

    #[test]
    fn is_slug_shape_rejects_too_long() {
        assert!(!is_slug_shape(&"a".repeat(65)));
    }

    #[test]
    fn is_slug_shape_rejects_uppercase_start() {
        assert!(!is_slug_shape("Abc"));
    }

    #[test]
    fn is_slug_shape_accepts_digit_start() {
        assert!(is_slug_shape("1abc"));
    }

    #[test]
    fn is_slug_shape_accepts_single_char() {
        assert!(is_slug_shape("a"));
    }

    #[test]
    fn org_resolve_error_display_not_found() {
        let err = OrgResolveError::NotFound("test".into());
        let msg = format!("{err}");
        assert!(msg.contains("not found"));
    }

    #[test]
    fn org_resolve_error_display_ambiguous() {
        let err = OrgResolveError::Ambiguous {
            input: "acme".into(),
            candidates: vec![candidate("1", "a", "A"), candidate("2", "b", "B")],
        };
        let msg = format!("{err}");
        assert!(msg.contains("ambiguous"));
    }

    #[test]
    fn org_resolve_error_display_api() {
        let err = OrgResolveError::Api(anyhow::anyhow!("connection refused"));
        let msg = format!("{err}");
        assert!(msg.contains("connection refused"));
    }

    #[test]
    fn org_resolve_error_source_api_has_inner() {
        let inner = anyhow::anyhow!("inner");
        let err = OrgResolveError::Api(inner);
        assert!(std::error::Error::source(&err).is_some());
    }

    #[test]
    fn org_resolve_error_source_not_found_is_none() {
        let err = OrgResolveError::NotFound("x".into());
        assert!(std::error::Error::source(&err).is_none());
    }

    #[tokio::test]
    async fn display_name_resolution_reuses_cached_org_list() {
        let mut lookup = FakeOrgLookup {
            orgs: vec![
                candidate("org-1", "chrono-ai", "Chrono AI"),
                candidate("org-2", "nyxid-labs", "NyxID Labs"),
            ],
            ..Default::default()
        };
        let mut cache = None;

        let first = resolve_fake(&mut lookup, &mut cache, "Chrono AI")
            .await
            .expect("resolve first display name");
        let second = resolve_fake(&mut lookup, &mut cache, "NyxID Labs")
            .await
            .expect("resolve second display name");

        assert_eq!(first, "org-1");
        assert_eq!(second, "org-2");
        assert_eq!(lookup.list_calls, 1);
    }
}
