use std::path::{Component, Path, PathBuf};

use axum::{
    body::Body,
    extract::{Path as AxumPath, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;

use crate::AppState;

const HTML_FILE: &str = "credential-accept.html";
const RELEASE_MANIFEST_RELATIVE: &str = "../release-integrity/releases.json";
const SCRIPT_ROLE_ATTR: &str = r#"data-nyx-integrity-role="credential_accept_script""#;

#[derive(Debug, Deserialize)]
struct ReleaseIntegrityManifest {
    artifacts: Vec<ReleaseIntegrityArtifact>,
}

#[derive(Debug, Deserialize)]
struct ReleaseIntegrityArtifact {
    role: String,
    path: String,
    content_type: String,
    sha384_sri: String,
}

struct CredentialAcceptAssets {
    html: String,
    script_sri: Vec<String>,
    artifacts: Vec<ReleaseIntegrityArtifact>,
}

fn unavailable(message: &'static str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        message,
    )
        .into_response()
}

fn dist_root(state: &AppState) -> PathBuf {
    PathBuf::from(&state.config.credential_accept_dist_dir)
}

fn release_manifest_path(root: &Path) -> PathBuf {
    root.join(RELEASE_MANIFEST_RELATIVE)
}

fn script_tag_for_src<'a>(html: &'a str, src: &str) -> Option<&'a str> {
    let src_attr = format!(r#"src="{src}""#);
    let mut offset = 0;
    while let Some(start_rel) = html[offset..].find("<script") {
        let start = offset + start_rel;
        let end_rel = html[start..].find("</script>")?;
        let end = start + end_rel + "</script>".len();
        let tag = &html[start..end];
        if tag.contains(&src_attr) {
            return Some(tag);
        }
        offset = end;
    }
    None
}

fn load_assets(root: &Path) -> Result<CredentialAcceptAssets, &'static str> {
    let manifest_bytes =
        std::fs::read(release_manifest_path(root)).map_err(|_| "release manifest unavailable")?;
    let manifest: ReleaseIntegrityManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|_| "release manifest invalid")?;
    let html = std::fs::read_to_string(root.join(HTML_FILE))
        .map_err(|_| "credential accept HTML missing")?;

    let script_artifacts: Vec<&ReleaseIntegrityArtifact> = manifest
        .artifacts
        .iter()
        .filter(|artifact| artifact.role == "credential_accept_script")
        .collect();
    let mut script_sri: Vec<String> = script_artifacts
        .iter()
        .map(|artifact| artifact.sha384_sri.clone())
        .collect();
    script_sri.sort();
    script_sri.dedup();
    if script_sri.is_empty() {
        return Err("release manifest has no credential accept scripts");
    }

    for artifact in script_artifacts {
        let Some(tag) = script_tag_for_src(&html, &artifact.path) else {
            return Err("credential accept HTML is missing script SRI");
        };
        if !artifact.sha384_sri.starts_with("sha384-")
            || !tag.contains(&format!("integrity=\"{}\"", artifact.sha384_sri))
        {
            return Err("credential accept HTML is missing script SRI");
        }
        if !tag.contains(SCRIPT_ROLE_ATTR) {
            return Err("credential accept HTML is missing script metadata");
        }
    }
    if html.contains("<script") && html.contains("src=\"/assets/") {
        return Err("credential accept HTML references an unscoped asset path");
    }

    Ok(CredentialAcceptAssets {
        html,
        script_sri,
        artifacts: manifest.artifacts,
    })
}

fn csp(script_sri: &[String]) -> String {
    let scripts = script_sri
        .iter()
        .map(|sri| format!(" '{sri}'"))
        .collect::<String>();
    format!(
        "default-src 'none'; base-uri 'none'; object-src 'none'; frame-ancestors 'none'; form-action 'none'; script-src 'self'{scripts}; connect-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:"
    )
}

fn html_response(assets: CredentialAcceptAssets) -> Response {
    let csp = csp(&assets.script_sri);
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
            (header::CONTENT_SECURITY_POLICY, csp.as_str()),
        ],
        assets.html,
    )
        .into_response()
}

/// GET /nodes/{node_id}/credentials/pending/{pending_id}/accept
pub async fn accept_page(State(state): State<AppState>) -> Response {
    match load_assets(&dist_root(&state)) {
        Ok(assets) => html_response(assets),
        Err(message) => unavailable(message),
    }
}

/// GET /nodes/credentials/pending/{pending_id}/fan-out/accept
pub async fn fan_out_accept_page(State(state): State<AppState>) -> Response {
    accept_page(State(state)).await
}

fn safe_asset_file(file: &str) -> Option<&str> {
    let path = Path::new(file);
    if path.is_absolute() {
        return None;
    }
    for component in path.components() {
        if !matches!(component, Component::Normal(_)) {
            return None;
        }
    }
    Some(file)
}

fn asset_artifact<'a>(
    assets: &'a CredentialAcceptAssets,
    file: &str,
) -> Option<&'a ReleaseIntegrityArtifact> {
    let expected = format!("/credential-accept/assets/{file}");
    assets
        .artifacts
        .iter()
        .find(|artifact| artifact.path == expected && artifact.role == "credential_accept_script")
}

/// GET /credential-accept/assets/{file}
pub async fn asset(State(state): State<AppState>, AxumPath(file): AxumPath<String>) -> Response {
    let Some(file) = safe_asset_file(&file) else {
        return (StatusCode::NOT_FOUND, Body::empty()).into_response();
    };
    let root = dist_root(&state);
    let Ok(assets) = load_assets(&root) else {
        return unavailable("release manifest unavailable");
    };
    let Some(artifact) = asset_artifact(&assets, file) else {
        return (StatusCode::NOT_FOUND, Body::empty()).into_response();
    };
    let path = root.join("assets").join(file);
    let Ok(bytes) = std::fs::read(path) else {
        return (StatusCode::NOT_FOUND, Body::empty()).into_response();
    };

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, artifact.content_type.as_str()),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        bytes,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::to_bytes, routing::get};
    use base64::Engine;
    use sha2::{Digest, Sha384};
    use tempfile::TempDir;
    use tower::ServiceExt;

    fn sri(bytes: &[u8]) -> (String, String) {
        let digest = Sha384::digest(bytes);
        (
            format!(
                "sha384-{}",
                base64::engine::general_purpose::STANDARD.encode(digest)
            ),
            hex::encode(digest),
        )
    }

    fn fixture() -> (TempDir, String) {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("credential-accept");
        let assets = root.join("assets");
        let release = tmp.path().join("release-integrity");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::create_dir_all(&release).unwrap();
        let script_bytes = b"globalThis.__nyxidAccept = true;";
        std::fs::write(assets.join("credential-accept-test.js"), script_bytes).unwrap();
        let (script_sri, script_hex) = sri(script_bytes);
        let html = format!(
            r#"<!doctype html><html><head><title>Accept</title></head><body><div id="credential-accept-root"></div><script type="module" data-nyx-integrity-role="credential_accept_script" src="/credential-accept/assets/credential-accept-test.js" integrity="{script_sri}" crossorigin="anonymous"></script></body></html>"#
        );
        std::fs::write(root.join(HTML_FILE), html.as_bytes()).unwrap();
        let (html_sri, html_hex) = sri(html.as_bytes());
        let manifest = serde_json::json!({
            "schema_version": "nyxid.release-integrity.v1",
            "app_version": "0.0.0",
            "git_commit": "test",
            "generated_at": "2026-06-05T00:00:00Z",
            "credential_accept": { "fingerprint_sha384_hex": script_hex },
            "artifacts": [
                {
                    "role": "credential_accept_html",
                    "path": "/credential-accept/credential-accept.html",
                    "content_type": "text/html; charset=utf-8",
                    "size_bytes": html.len(),
                    "sha384_sri": html_sri,
                    "sha384_hex": html_hex
                },
                {
                    "role": "credential_accept_script",
                    "path": "/credential-accept/assets/credential-accept-test.js",
                    "content_type": "text/javascript; charset=utf-8",
                    "size_bytes": script_bytes.len(),
                    "sha384_sri": script_sri,
                    "sha384_hex": script_hex
                }
            ]
        });
        std::fs::write(
            release.join("releases.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        (tmp, root.display().to_string())
    }

    async fn app(dist_dir: String) -> Router {
        let state = crate::test_utils::test_app_state_no_db().await;
        let mut state = state;
        state.config.credential_accept_dist_dir = dist_dir;
        Router::new()
            .route(
                "/nodes/{node_id}/credentials/pending/{pending_id}/accept",
                get(accept_page),
            )
            .route("/credential-accept/assets/{*file}", get(asset))
            .with_state(state)
    }

    #[tokio::test]
    async fn accept_route_returns_standalone_html_with_csp_and_sri() {
        let (_tmp, dist_dir) = fixture();
        let response = app(dist_dir)
            .await
            .oneshot(
                axum::http::Request::builder()
                    .uri("/nodes/node-1/credentials/pending/pending-1/accept")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let csp = response
            .headers()
            .get(header::CONTENT_SECURITY_POLICY)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(csp.starts_with("default-src 'none'; base-uri 'none'; object-src 'none'"));
        assert!(csp.contains("script-src 'self' 'sha384-"));
        assert!(csp.contains("connect-src 'self'"));
        assert!(csp.contains("style-src 'self' 'unsafe-inline'"));
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("credential-accept-root"));
        assert!(!html.contains("root\"></div><script type=\"module\" src=\"/src/main"));
        assert!(html.contains("integrity=\"sha384-"));
        assert!(html.contains(SCRIPT_ROLE_ATTR));
        assert!(!html.contains("<script>"));
    }

    #[tokio::test]
    async fn missing_manifest_fails_closed() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("credential-accept")).unwrap();
        std::fs::write(
            tmp.path().join("credential-accept").join(HTML_FILE),
            "<html></html>",
        )
        .unwrap();
        let response = app(tmp.path().join("credential-accept").display().to_string())
            .await
            .oneshot(
                axum::http::Request::builder()
                    .uri("/nodes/node-1/credentials/pending/pending-1/accept")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn accept_route_rejects_html_missing_script_integrity_role() {
        let (_tmp, dist_dir) = fixture();
        let html_path = Path::new(&dist_dir).join(HTML_FILE);
        let html = std::fs::read_to_string(&html_path).unwrap();
        std::fs::write(
            &html_path,
            html.replace(r#" data-nyx-integrity-role="credential_accept_script""#, ""),
        )
        .unwrap();

        let response = app(dist_dir)
            .await
            .oneshot(
                axum::http::Request::builder()
                    .uri("/nodes/node-1/credentials/pending/pending-1/accept")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            String::from_utf8(body.to_vec()).unwrap(),
            "credential accept HTML is missing script metadata"
        );
    }

    #[tokio::test]
    async fn asset_route_serves_only_manifest_listed_script() {
        let (_tmp, dist_dir) = fixture();
        let app = app(dist_dir).await;
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/credential-accept/assets/credential-accept-test.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "public, max-age=31536000, immutable"
        );

        let missing = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/credential-accept/assets/not-listed.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    }
}
