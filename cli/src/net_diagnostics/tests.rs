use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[tokio::test]
async fn refused_connection_is_connect_and_redacts_endpoint() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let client = crate::tls::client_builder()
        .unwrap()
        .no_proxy()
        .build()
        .unwrap();
    let error = client
        .get(format!(
            "http://user:secret@{address}/api?token=QUERY_SECRET#FRAGMENT_SECRET"
        ))
        .send()
        .await
        .unwrap_err();
    let diagnostic = Diagnostic::reqwest(&error, None, None);
    assert_eq!(diagnostic.stage, Stage::Connect);
    assert_eq!(
        diagnostic.endpoint.unwrap(),
        format!("http://{address}/api")
    );
    let error = anyhow::Error::from(error).context("request failed");
    for json in [false, true] {
        let output = crate::error_format::render_error(&error, json);
        for secret in ["secret", "QUERY_SECRET", "FRAGMENT_SECRET", "user:"] {
            assert!(!output.contains(secret), "{output}");
        }
        assert!(output.contains("connect"));
        assert!(output.contains("hint"));
    }
}

#[tokio::test]
async fn silent_server_reports_request_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        std::future::pending::<()>().await;
    });
    let error = crate::tls::client_builder()
        .unwrap()
        .no_proxy()
        .timeout(std::time::Duration::from_millis(100))
        .build()
        .unwrap()
        .get(format!("http://{address}"))
        .send()
        .await
        .unwrap_err();
    let diagnostic = Diagnostic::reqwest(&error, None, None);
    assert_eq!(diagnostic.stage, Stage::Request);
    assert_eq!(diagnostic.timeout, Some("request"));
    server.abort();
}

#[tokio::test]
async fn untrusted_certificate_reports_tls() {
    let server = crate::tls::tests::Server::start("localhost", false).await;
    let error = crate::tls::tests::client(&crate::tls::tests::configured(None))
        .get(server.url())
        .send()
        .await
        .unwrap_err();
    let diagnostic = Diagnostic::reqwest(&error, None, None);
    assert_eq!(diagnostic.stage, Stage::Tls, "{diagnostic:?}");
    assert!(diagnostic.hint.contains("NYXID_CA_CERT"));
    assert!(diagnostic.hint.contains("TLS-inspecting proxy"));
    assert!(diagnostic.causes.windows(2).all(|pair| pair[0] != pair[1]));
    assert_eq!(
        diagnostic
            .causes
            .iter()
            .filter(|cause| cause.contains("UnknownIssuer"))
            .count(),
        1
    );
}

#[tokio::test]
async fn failed_connect_tunnels_report_proxy_without_credentials() {
    for status in [403, 407, 502] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(stream.read_u8().await.unwrap());
            }
            stream
                .write_all(
                    format!("HTTP/1.1 {status} Rejected\r\nContent-Length: 0\r\n\r\n").as_bytes(),
                )
                .await
                .unwrap();
        });
        let error = crate::tls::client_builder()
            .unwrap()
            .proxy(reqwest::Proxy::all(format!("http://user:PROXY_SECRET@{address}")).unwrap())
            .build()
            .unwrap()
            .get("https://example.invalid/api?device_code=DEVICE_SECRET")
            .send()
            .await
            .unwrap_err();
        let diagnostic = Diagnostic::reqwest(&error, None, None);
        assert_eq!(diagnostic.stage, Stage::Proxy, "{diagnostic:?}");
        for output in [
            diagnostic.text(),
            serde_json::to_string(&diagnostic).unwrap(),
        ] {
            assert!(!output.contains("PROXY_SECRET"));
            assert!(!output.contains("DEVICE_SECRET"));
        }
        server.await.unwrap();
    }
}

#[test]
fn sanitization_removes_url_and_header_secrets_and_bounds_causes() {
    assert_eq!(
        sanitize_endpoint("https://user:secret@host:8443/path?a=token#fragment").unwrap(),
        "https://host:8443/path"
    );
    assert_eq!(
        proxy_origin("http://user:secret@host:8123/path?token=secret"),
        "http://host:8123"
    );
    assert_eq!(proxy_origin("user:secret@host:8123"), "http://host:8123");
    for raw in [
        "at https://user:secret@host/path?secret=hidden#fragment",
        "Authorization: Bearer secret",
        "refresh_token=secret",
        "password: secret",
        "device_code=secret",
    ] {
        let output = sanitize(raw);
        assert!(!output.contains("secret"), "{output}");
        assert!(!output.contains("hidden"), "{output}");
    }
    let diagnostic = Diagnostic::new(Stage::Request, None, None, &"é".repeat(1000));
    assert_eq!(diagnostic.causes[0].chars().count(), 300);
    assert!(diagnostic.causes.len() <= 8);
}

#[tokio::test]
async fn stalled_tls_handshake_reports_connect_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        std::future::pending::<()>().await;
    });
    let error = crate::tls::client_builder()
        .unwrap()
        .no_proxy()
        .connect_timeout(std::time::Duration::from_millis(100))
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap()
        .get(format!("https://{address}"))
        .send()
        .await
        .unwrap_err();
    let diagnostic = Diagnostic::reqwest(&error, None, None);
    assert_eq!(diagnostic.stage, Stage::Connect);
    assert_eq!(diagnostic.timeout, Some("connect"));
    server.abort();
}

#[test]
fn hints_are_neutral_unless_a_caller_supplies_context() {
    let response = Diagnostic::new(Stage::Response, None, Some(503), "response failed");
    assert!(!response.hint.contains("NyxID"));
    assert!(!response.hint.contains("--base-url"));
    assert!(response.clone().for_login().hint.contains("--base-url"));
    let github = response.for_github();
    assert!(github.hint.contains("GitHub API status and rate limits"));
    assert!(!github.hint.contains("--base-url"));
    let validation = Diagnostic::new(Stage::Validation, None, None, "invalid value");
    assert!(!validation.hint.contains("verification URL"));
    assert!(validation.for_login().hint.contains("verification URL"));
    for variable in ["NYXID_CA_CERT", "SSL_CERT_FILE", "SSL_CERT_DIR"] {
        let config = Diagnostic::new(
            Stage::Config,
            None,
            None,
            &format!("{variable} (/some/long/path): missing file"),
        );
        assert_eq!(config.hint, format!("Fix or unset {variable}, then retry."));
        assert!(!config.hint.contains("/some/long/path"));
    }
}

#[test]
fn tls_hints_distinguish_ca_hostname_validity_and_other_errors() {
    use rustls::CertificateError::*;
    for error in [UnknownIssuer, BadSignature] {
        let hint = tls_hint(&rustls::Error::InvalidCertificate(error));
        assert!(hint.contains("NYXID_CA_CERT"));
        assert!(hint.contains("SSL_CERT_FILE"));
        assert!(hint.contains("TLS-inspecting proxy"));
    }
    for error in [
        NotValidForName,
        NotValidForNameContext {
            expected: "example.com".try_into().unwrap(),
            presented: vec!["different.example.com".into()],
        },
    ] {
        assert_eq!(
            tls_hint(&rustls::Error::InvalidCertificate(error)),
            HOST_HINT
        );
    }
    let now = pki_types::UnixTime::since_unix_epoch(std::time::Duration::from_secs(100));
    for error in [
        Expired,
        ExpiredContext {
            time: now,
            not_after: now,
        },
    ] {
        let hint = tls_hint(&rustls::Error::InvalidCertificate(error));
        assert!(hint.contains("expired"));
        assert!(hint.contains("system clock"));
    }
    for error in [
        NotValidYet,
        NotValidYetContext {
            time: now,
            not_before: now,
        },
    ] {
        let hint = tls_hint(&rustls::Error::InvalidCertificate(error));
        assert!(hint.contains("not yet valid"));
        assert!(hint.contains("system clock"));
    }
    assert!(!tls_hint(&rustls::Error::InvalidCertificate(BadEncoding)).contains("NYXID_CA_CERT"));
    assert!(
        tls_hint(&rustls::Error::General("handshake failed".into()))
            .contains("server TLS configuration")
    );
}

#[tokio::test]
async fn wrong_host_handshake_selects_hostname_hint_instead_of_ca_hint() {
    let server = crate::tls::tests::Server::start("other.example", false).await;
    let ca = server.ca_file();
    let error = crate::tls::tests::client(&crate::tls::tests::configured(Some(ca.path().into())))
        .get(server.url())
        .send()
        .await
        .unwrap_err();
    let diagnostic = Diagnostic::reqwest(&error, None, None);
    assert_eq!(diagnostic.stage, Stage::Tls);
    assert_eq!(diagnostic.hint, HOST_HINT);
    assert!(!diagnostic.hint.contains("NYXID_CA_CERT"));
    assert!(diagnostic.for_login().hint.contains("--base-url"));
}

#[test]
fn no_proxy_patterns_are_visible_bounded_and_without_control_characters() {
    assert_eq!(
        bypass_list("localhost,\n.example.com,\r\t127.0.0.1"),
        "localhost,.example.com,127.0.0.1"
    );
    assert_eq!(bypass_list(&"é".repeat(400)).chars().count(), 300);
}
