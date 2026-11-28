use axum::{
    Json,
    body::Body,
    http::{HeaderMap, Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use futures::{StreamExt as _, stream};
use serde::Deserialize;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use url::Url;

const MAX_REFUSAL_BYTES: usize = 8192;

const METADATA_HEADERS: &[&str] = &[
    "x-oauth-scopes",
    "x-accepted-oauth-scopes",
    "x-accepted-github-permissions",
    "x-github-request-id",
    "x-ratelimit-remaining",
];

pub(super) fn verified_route(catalog_slug: Option<&str>, bearer: bool, destination: &Url) -> bool {
    catalog_slug == Some("api-github")
        && bearer
        && destination.scheme() == "https"
        && destination.host_str() == Some("api.github.com")
        && destination.port_or_known_default() == Some(443)
        && destination.username().is_empty()
        && destination.password().is_none()
}

pub(super) fn forward_header(name: &str) -> bool {
    METADATA_HEADERS.contains(&name)
}

pub(super) async fn write_scope_refusal(
    response: Response,
    frontend: &str,
    service_id: &str,
    stream_completed: Option<&AtomicBool>,
) -> Response {
    let Some(remediation_url) = manage_url(frontend, service_id) else {
        return response;
    };
    let (mut parts, body) = response.into_parts();
    let mut relay = body.into_data_stream();
    let mut inspected = bytes::BytesMut::new();
    // Poll the ordinary relay so its accounting and idle timeout also apply to
    // the inspected bytes. Pass through anything that exceeds the error bound.
    while let Some(chunk) = relay.next().await {
        match chunk {
            Ok(chunk) if chunk.len() <= MAX_REFUSAL_BYTES - inspected.len() => {
                inspected.extend_from_slice(&chunk);
            }
            remainder => {
                let prefix = stream::iter([Ok(inspected.freeze()), remainder]);
                return Response::from_parts(parts, Body::from_stream(prefix.chain(relay)));
            }
        }
    }
    let bytes = inspected.freeze();
    if stream_completed.is_some_and(|completed| !completed.load(Ordering::Relaxed))
        || !generic_refusal(parts.status, &bytes)
    {
        return Response::from_parts(parts, Body::from(bytes));
    }

    parts.status = StatusCode::FORBIDDEN;
    for name in [
        header::CONTENT_LENGTH,
        header::CONTENT_ENCODING,
        header::CONTENT_LANGUAGE,
        header::CONTENT_RANGE,
        header::ETAG,
        header::LAST_MODIFIED,
    ] {
        parts.headers.remove(name);
    }
    parts.headers.insert(
        header::CONTENT_TYPE,
        "application/problem+json".parse().unwrap(),
    );
    parts
        .headers
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    let body = Json(json!({
        "code": "insufficient_scope",
        "status": 403,
        "detail": "public_repo",
        "message": "GitHub connection requires public_repo for this repository write",
        "connection_id": service_id,
        "remediation_url": remediation_url,
    }))
    .into_response()
    .into_body();
    Response::from_parts(parts, body)
}

pub(super) fn missing_public_write_scope(
    method: &Method,
    path: &str,
    status: StatusCode,
    headers: &HeaderMap,
) -> bool {
    if method != Method::POST
        || !matches!(status, StatusCode::NOT_FOUND | StatusCode::FORBIDDEN)
        || !one_header(headers, "content-type")
            .is_some_and(crate::services::content_type::is_json_content_type)
        || one_header(headers, "x-github-request-id").is_none_or(str::is_empty)
        || headers.contains_key(header::RETRY_AFTER)
        || headers.contains_key("x-github-sso")
        || headers.contains_key("x-accepted-github-permissions")
        || (status == StatusCode::FORBIDDEN
            && one_header(headers, "x-ratelimit-remaining")
                .and_then(|value| value.parse::<u64>().ok())
                .is_none_or(|remaining| remaining == 0))
    {
        return false;
    }
    let Some((repository, endpoint)) = path.strip_prefix("/repos/").and_then(|path| {
        let (owner, rest) = path.split_once('/')?;
        let (repository, endpoint) = rest.split_once('/')?;
        (!owner.is_empty()).then_some((repository, endpoint))
    }) else {
        return false;
    };
    !repository.is_empty()
        && matches!(endpoint, "git/refs" | "issues" | "pulls")
        && scopes(headers, "x-accepted-oauth-scopes").is_some_and(|scopes| {
            matches!(
                scopes.as_slice(),
                ["public_repo"] | ["repo", "public_repo"] | ["public_repo", "repo"]
            )
        })
        && scopes(headers, "x-oauth-scopes").is_some_and(|scopes| {
            !scopes
                .iter()
                .any(|scope| matches!(*scope, "public_repo" | "repo"))
        })
}

fn one_header<'a>(headers: &'a HeaderMap, name: &'static str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let first = values.next()?.to_str().ok()?;
    values.next().is_none().then_some(first)
}

fn scopes<'a>(headers: &'a HeaderMap, name: &'static str) -> Option<Vec<&'a str>> {
    let value = one_header(headers, name)?.trim();
    if value.is_empty() {
        return Some(Vec::new());
    }
    let scopes: Vec<_> = value.split(',').map(str::trim).collect();
    (scopes.len() <= 100
        && scopes.iter().all(|scope| {
            (1..=64).contains(&scope.len())
                && scope
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'-'))
        }))
    .then_some(scopes)
}

fn generic_refusal(status: StatusCode, bytes: &[u8]) -> bool {
    #[derive(Deserialize)]
    struct GithubError<'a> {
        message: &'a str,
        errors: Option<serde::de::IgnoredAny>,
    }
    serde_json::from_slice::<GithubError<'_>>(bytes).is_ok_and(|error| {
        error.errors.is_none()
            && matches!(
                (status, error.message),
                (StatusCode::NOT_FOUND, "Not Found") | (StatusCode::FORBIDDEN, "Forbidden")
            )
    })
}

fn manage_url(frontend: &str, service_id: &str) -> Option<Url> {
    let mut url = Url::parse(frontend).ok()?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    // Public service IDs are UUIDs. Prevent legacy or malformed IDs from
    // identifying any other console path.
    uuid::Uuid::parse_str(service_id).ok()?;
    url.set_query(None);
    url.set_fragment(None);
    url.path_segments_mut()
        .ok()?
        .clear()
        .push("keys")
        .push(service_id);
    Some(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    const SERVICE: &str = "05ceabf7-ad92-42d0-9be7-047aa3f869e7";
    const PATH: &str = "/repos/example/project/git/refs";

    fn evidence() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
        headers.insert("x-github-request-id", "synthetic-request".parse().unwrap());
        headers.insert("x-oauth-scopes", "read:user".parse().unwrap());
        headers.insert(
            "x-accepted-oauth-scopes",
            "public_repo, repo".parse().unwrap(),
        );
        headers.insert("x-ratelimit-remaining", "97".parse().unwrap());
        headers
    }

    #[test]
    fn only_catalog_github_bearer_responses_can_share_metadata() {
        let destination = Url::parse("https://api.github.com/user").unwrap();
        assert!(verified_route(Some("api-github"), true, &destination));
        assert!(!verified_route(Some("api-github-pat"), true, &destination));
        assert!(!verified_route(None, true, &destination));
        assert!(!verified_route(Some("api-github"), false, &destination));
        for address in [
            "http://api.github.com/user",
            "https://api.github.com.example.test/user",
            "https://api.github.com:444/user",
        ] {
            let destination = Url::parse(address).unwrap();
            assert!(!verified_route(Some("api-github"), true, &destination));
        }
        for name in METADATA_HEADERS {
            assert!(forward_header(name));
        }
        assert!(!forward_header("authorization"));
    }

    #[test]
    fn only_clear_scope_evidence_classifies_a_write_refusal() {
        let matches = |headers: &HeaderMap, status| {
            missing_public_write_scope(&Method::POST, PATH, status, headers)
        };
        let mut headers = evidence();
        assert!(matches(&headers, StatusCode::NOT_FOUND));
        headers.insert(
            "x-accepted-oauth-scopes",
            "repo, public_repo".parse().unwrap(),
        );
        assert!(matches(&headers, StatusCode::NOT_FOUND));
        assert!(!missing_public_write_scope(
            &Method::GET,
            PATH,
            StatusCode::NOT_FOUND,
            &headers
        ));
        assert!(!missing_public_write_scope(
            &Method::POST,
            "/graphql",
            StatusCode::NOT_FOUND,
            &headers
        ));
        assert!(!matches(&headers, StatusCode::UNAUTHORIZED));
        headers.insert("x-oauth-scopes", "repo".parse().unwrap());
        assert!(!matches(&headers, StatusCode::NOT_FOUND));
        headers.remove("x-oauth-scopes");
        assert!(!matches(&headers, StatusCode::NOT_FOUND));
        headers.insert("x-oauth-scopes", "".parse().unwrap());
        assert!(matches(&headers, StatusCode::NOT_FOUND));
        headers.append("x-oauth-scopes", "repo".parse().unwrap());
        assert!(!matches(&headers, StatusCode::NOT_FOUND));

        let mut headers = evidence();
        headers.insert("x-ratelimit-remaining", "0".parse().unwrap());
        assert!(!matches(&headers, StatusCode::FORBIDDEN));
        headers.insert(header::RETRY_AFTER, "1".parse().unwrap());
        assert!(!matches(&headers, StatusCode::NOT_FOUND));
        headers.remove(header::RETRY_AFTER);
        headers.insert(
            "x-accepted-github-permissions",
            "issues=write".parse().unwrap(),
        );
        assert!(!matches(&headers, StatusCode::NOT_FOUND));
        headers.remove("x-accepted-github-permissions");
        headers.remove("x-accepted-oauth-scopes");
        assert!(!matches(&headers, StatusCode::NOT_FOUND));
    }

    #[tokio::test]
    async fn sso_refusal_survives_projection_without_disclosing_the_sso_url() {
        let body = r#"{"message":"Forbidden"}"#;
        let status = StatusCode::FORBIDDEN;
        let mut headers = evidence();
        headers.insert(
            "x-github-sso",
            "required; url=https://github.com/orgs/fixture/sso"
                .parse()
                .unwrap(),
        );
        let classify = missing_public_write_scope(&Method::POST, PATH, status, &headers);
        let mut response = super::super::project_direct_response_headers(
            Response::builder().status(status),
            status,
            &headers,
            false,
            None,
            true,
        )
        .body(Body::from(body))
        .unwrap();
        if classify {
            response =
                write_scope_refusal(response, "https://console.example.test", SERVICE, None).await;
        }
        assert!(response.headers().contains_key("x-oauth-scopes"));
        assert!(!response.headers().contains_key("x-github-sso"));
        assert_eq!(response.status(), status);
        assert_eq!(to_bytes(response.into_body(), 8192).await.unwrap(), body);
    }

    #[tokio::test]
    async fn typed_scope_refusal_keeps_evidence_and_selects_exact_management_route() {
        let mut response = Response::new(Body::from(r#"{"message":"Not Found"}"#));
        *response.status_mut() = StatusCode::NOT_FOUND;
        *response.headers_mut() = evidence();
        response
            .headers_mut()
            .insert(header::CONTENT_LENGTH, "23".parse().unwrap());
        let response =
            write_scope_refusal(response, "https://console.example.test/", SERVICE, None).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(response.headers().contains_key("x-oauth-scopes"));
        assert!(!response.headers().contains_key(header::CONTENT_LENGTH));
        let body = to_bytes(response.into_body(), 8192).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["code"], "insufficient_scope");
        assert_eq!(json["detail"], "public_repo");
        assert_eq!(json["connection_id"], SERVICE);
        assert_eq!(
            json["remediation_url"],
            format!("https://console.example.test/keys/{SERVICE}")
        );
        assert!(manage_url("https://console.example.test/", "..").is_none());
    }

    #[tokio::test]
    async fn chunked_refusals_are_classified_only_after_a_small_complete_body() {
        use bytes::Bytes;
        use std::sync::Arc;

        let generic = Bytes::from_static(br#"{"message":"Not Found"}"#);
        let independent =
            Bytes::from_static(br#"{"message":"Not Found","errors":[{"code":"repository_rule"}]}"#);
        let large_chunk = Bytes::from(vec![b' '; MAX_REFUSAL_BYTES]);
        for (chunks, reaches_eof, expected_scope) in [
            (vec![generic.slice(..8), generic.slice(8..)], true, true),
            (vec![generic.clone()], false, false),
            (vec![independent], true, false),
            (
                vec![generic.clone(), large_chunk, Bytes::from_static(b"tail")],
                true,
                false,
            ),
        ] {
            let expected = chunks.concat();
            let stream_completed = Arc::new(AtomicBool::new(false));
            let inspector = stream_completed.clone();
            let stream = async_stream::stream! {
                for chunk in chunks {
                    yield Ok::<_, std::io::Error>(chunk);
                }
                if reaches_eof {
                    stream_completed.store(true, Ordering::Relaxed);
                }
            };
            let status = StatusCode::NOT_FOUND;
            let headers = evidence();
            assert!(missing_public_write_scope(
                &Method::POST,
                PATH,
                status,
                &headers
            ));
            let response = super::super::project_direct_response_headers(
                Response::builder().status(status),
                status,
                &headers,
                false,
                None,
                true,
            )
            .body(Body::from_stream(stream))
            .unwrap();
            assert!(!response.headers().contains_key(header::CONTENT_LENGTH));
            let response = write_scope_refusal(
                response,
                "https://console.example.test",
                SERVICE,
                Some(&inspector),
            )
            .await;
            if expected.len() > MAX_REFUSAL_BYTES {
                assert!(!inspector.load(Ordering::Relaxed));
            }
            let status = response.status();
            let body = to_bytes(response.into_body(), MAX_REFUSAL_BYTES * 2)
                .await
                .unwrap();
            if expected_scope {
                assert_eq!(status, StatusCode::FORBIDDEN);
                let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(body["code"], "insufficient_scope");
                assert_eq!(
                    body["remediation_url"],
                    format!("https://console.example.test/keys/{SERVICE}")
                );
            } else {
                assert_eq!(status, StatusCode::NOT_FOUND);
                assert_eq!(body.as_ref(), expected);
            }
        }
    }

    #[tokio::test]
    async fn preserves_independent_provider_refusals() {
        for body in [
            r#"{"message":"You have exceeded a secondary rate limit"}"#,
            r#"{"message":"Forbidden","errors":[{"code":"branch_protection"}]}"#,
        ] {
            let mut response = Response::new(Body::from(body));
            *response.status_mut() = StatusCode::FORBIDDEN;
            *response.headers_mut() = evidence();
            let response =
                write_scope_refusal(response, "https://console.example.test", SERVICE, None).await;
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            assert_eq!(
                to_bytes(response.into_body(), 8192).await.unwrap(),
                body.as_bytes()
            );
        }
    }
}
