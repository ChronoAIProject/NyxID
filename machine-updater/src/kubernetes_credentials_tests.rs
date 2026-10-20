use super::*;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

async fn fixture(response: ResponseTemplate) -> (MockServer, KubernetesCredentials) {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/computeMetadata/v1/instance/service-accounts/default/token",
        ))
        .and(header("Metadata-Flavor", "Google"))
        .respond_with(response)
        .mount(&server)
        .await;
    let credentials = KubernetesCredentials::metadata_for_test(
        format!(
            "{}/computeMetadata/v1/instance/service-accounts/default/token",
            server.uri()
        )
        .parse()
        .unwrap(),
        Duration::from_millis(150),
        Duration::from_millis(250),
        Duration::from_millis(40),
    )
    .unwrap();
    (server, credentials)
}
fn success(token: &str, ttl: u64) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .set_body_json(serde_json::json!({"access_token": token, "expires_in": ttl}))
}
async fn authorization(credentials: &KubernetesCredentials) -> Result<String> {
    let request = credentials
        .authorize(Client::new().get("https://kubernetes.invalid/version"))
        .await?
        .build()?;
    assert!(request.headers()[reqwest::header::AUTHORIZATION].is_sensitive());
    Ok(request.headers()[reqwest::header::AUTHORIZATION]
        .to_str()?
        .to_owned())
}
#[tokio::test]
async fn metadata_protocol_cache_and_concurrent_reuse() {
    // Pin the actual production constructor as well as exercising the local
    // endpoint protocol below; this performs no live metadata request.
    let production = KubernetesCredentials::google_metadata().unwrap();
    let Source::GoogleMetadata(source) = &production.source else {
        panic!()
    };
    assert_eq!(
        source.endpoint.as_str(),
        "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token"
    );
    let (server, credentials) = fixture(success("fixture-google-secret", 3599)).await;
    let results = futures::future::join_all((0..32).map(|_| authorization(&credentials))).await;
    assert!(
        results
            .into_iter()
            .all(|r| r.unwrap() == "Bearer fixture-google-secret")
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    assert_eq!(
        authorization(&credentials.clone()).await.unwrap(),
        "Bearer fixture-google-secret"
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    assert!(!format!("{credentials:?}").contains("fixture-google-secret"));
}
#[tokio::test]
async fn metadata_early_refresh_and_expired_token_never_reused() {
    let (server, credentials) = fixture(success("first", 1)).await;
    assert_eq!(authorization(&credentials).await.unwrap(), "Bearer first");
    tokio::time::sleep(Duration::from_millis(920)).await;
    server.reset().await;
    Mock::given(method("GET"))
        .respond_with(success("second", 1))
        .mount(&server)
        .await;
    assert_eq!(authorization(&credentials).await.unwrap(), "Bearer second");
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    tokio::time::sleep(Duration::from_millis(1050)).await;
    server.reset().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503).set_body_string("second"))
        .mount(&server)
        .await;
    let error = authorization(&credentials).await.unwrap_err();
    assert!(!format!("{error:#?}").contains("second"));
}
#[tokio::test]
async fn metadata_failed_refresh_collapses_and_recovers() {
    let (server, credentials) = fixture(success("before-failure", 1)).await;
    assert_eq!(
        authorization(&credentials).await.unwrap(),
        "Bearer before-failure"
    );
    tokio::time::sleep(Duration::from_millis(920)).await;
    server.reset().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503).set_body_string("secret-body"))
        .mount(&server)
        .await;
    let results = futures::future::join_all((0..32).map(|_| authorization(&credentials))).await;
    for result in results {
        assert!(!format!("{:#?}", result.unwrap_err()).contains("secret-body"));
    }
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    tokio::time::sleep(Duration::from_millis(50)).await;
    server.reset().await;
    Mock::given(method("GET"))
        .respond_with(success("recovered", 3599))
        .mount(&server)
        .await;
    assert_eq!(
        authorization(&credentials).await.unwrap(),
        "Bearer recovered"
    );
}
#[tokio::test]
async fn metadata_timeout_wait_is_bounded_and_retries() {
    let (server, credentials) =
        fixture(success("delayed", 3599).set_delay(Duration::from_millis(500))).await;
    let start = tokio::time::Instant::now();
    assert!(
        futures::future::join_all((0..24).map(|_| authorization(&credentials)))
            .await
            .into_iter()
            .all(|r| r.is_err())
    );
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    tokio::time::sleep(Duration::from_millis(50)).await;
    server.reset().await;
    Mock::given(method("GET"))
        .respond_with(success("retry", 3599))
        .mount(&server)
        .await;
    assert_eq!(authorization(&credentials).await.unwrap(), "Bearer retry");
}
#[tokio::test]
async fn metadata_rejects_invalid_tokens_ttls_bodies_and_redirects() {
    for body in [
        r#"{"access_token":"","expires_in":3599}"#.to_owned(),
        r#"{"access_token":"secret\r\ninjection","expires_in":3599}"#.to_owned(),
        r#"{"access_token":"secret space","expires_in":3599}"#.to_owned(),
        r#"{"access_token":"secret","expires_in":0}"#.to_owned(),
        r#"{"access_token":"secret","expires_in":-1}"#.to_owned(),
        r#"{"access_token":"secret","expires_in":1.5}"#.to_owned(),
        r#"{"access_token":"secret","expires_in":18446744073709551615}"#.to_owned(),
        r#"{"access_token":"secret","expires_in":"3599"}"#.to_owned(),
        r#"{"access_token":"secret"}"#.to_owned(),
        "secret invalid JSON".to_owned(),
        "x".repeat(MAX_METADATA_RESPONSE + 1),
        serde_json::json!({"access_token":"x".repeat(MAX_METADATA_TOKEN + 1),"expires_in":3599})
            .to_string(),
    ] {
        let (_server, credentials) =
            fixture(ResponseTemplate::new(200).set_body_string(body)).await;
        let error = authorization(&credentials).await.unwrap_err();
        assert!(!format!("{error:#?}").contains("secret"));
    }
    let target = MockServer::start().await;
    let (_server, credentials) =
        fixture(ResponseTemplate::new(302).insert_header("Location", target.uri())).await;
    assert!(authorization(&credentials).await.is_err());
    assert!(target.received_requests().await.unwrap().is_empty());
}
#[tokio::test]
async fn metadata_acquisition_includes_mutex_wait_and_conservative_elapsed_lifetime() {
    let (_server, credentials) = fixture(success("too-late", 1)).await;
    let Source::GoogleMetadata(source) = &credentials.source else {
        panic!()
    };
    let held = source.state.lock().await;
    let start = tokio::time::Instant::now();
    assert!(authorization(&credentials).await.is_err());
    assert!(start.elapsed() < Duration::from_secs(1));
    drop(held);
    // A token whose lifetime elapsed in transit is never returned even on first acquisition.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(success("too-late", 1).set_delay(Duration::from_millis(1100)))
        .mount(&server)
        .await;
    let credentials = KubernetesCredentials::metadata_for_test(
        server.uri().parse().unwrap(),
        Duration::from_secs(2),
        Duration::from_secs(3),
        Duration::from_millis(40),
    )
    .unwrap();
    assert!(authorization(&credentials).await.is_err());
}
#[tokio::test]
async fn shared_file_source_rotates_and_protects_reads_without_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("token");
    std::fs::write(&path, "first\n").unwrap();
    let credentials = KubernetesCredentials::token_file(path.clone()).unwrap();
    assert_eq!(authorization(&credentials).await.unwrap(), "Bearer first");
    let replacement = dir.path().join("new");
    std::fs::write(&replacement, "second").unwrap();
    std::fs::rename(replacement, &path).unwrap();
    assert_eq!(authorization(&credentials).await.unwrap(), "Bearer second");
    for value in [
        String::new(),
        "bad\ninterior".into(),
        "x".repeat(MAX_FILE_TOKEN + 1),
    ] {
        std::fs::write(&path, value).unwrap();
        assert!(authorization(&credentials).await.is_err());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        std::fs::write(&path, "protected").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(authorization(&credentials).await.is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = dir.path().join("projected");
        symlink(&path, &link).unwrap();
        assert_eq!(
            authorization(&KubernetesCredentials::token_file(link).unwrap())
                .await
                .unwrap(),
            "Bearer protected"
        );
    }
    std::fs::remove_file(&path).unwrap();
    assert!(authorization(&credentials).await.is_err());
    assert!(KubernetesCredentials::token_file("relative".into()).is_err());
    assert!(!format!("{credentials:?}").contains(path.to_str().unwrap()));
}

#[tokio::test]
async fn metadata_bounds_chunked_response_without_content_length() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut headers = Vec::new();
        while !headers.ends_with(b"\r\n\r\n") {
            headers.push(stream.read_u8().await.unwrap());
        }
        let bytes = "x".repeat(MAX_METADATA_RESPONSE + 1);
        let response = format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{bytes}\r\n0\r\n\r\n",
            bytes.len()
        );
        let _ = stream.write_all(response.as_bytes()).await;
    });
    let credentials = KubernetesCredentials::metadata_for_test(
        endpoint.parse().unwrap(),
        Duration::from_secs(1),
        Duration::from_secs(2),
        Duration::from_millis(40),
    )
    .unwrap();
    assert!(authorization(&credentials).await.is_err());
    server.await.unwrap();
}
