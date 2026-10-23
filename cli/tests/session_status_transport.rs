use base64::Engine;
use std::sync::{Arc, Mutex};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[derive(Clone, Copy)]
enum Body {
    Stalled,
    Truncated,
    InvalidJson,
}

async fn check_body_failure(body: Body, expected: &str) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let destination = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let server = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let (mut stream, _) = accepted.unwrap();
                    let captured = captured.clone();
                    connections.spawn(async move {
                        let mut request = Vec::new();
                        let mut buffer = [0; 1024];
                        while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                            let count = stream.read(&mut buffer).await.unwrap();
                            if count == 0 {
                                return;
                            }
                            request.extend_from_slice(&buffer[..count]);
                        }
                        let first_line = String::from_utf8_lossy(&request)
                            .lines().next().unwrap().to_owned();
                        captured.lock().unwrap().push(first_line.clone());
                        if first_line != "GET /api/v1/sessions HTTP/1.1" {
                            stream.write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                            return;
                        }
                        let response = match body {
                            Body::InvalidJson => b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nConnection: close\r\n\r\n[".as_slice(),
                            _ => b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n[".as_slice(),
                        };
                        stream.write_all(response).await.unwrap();
                        if matches!(body, Body::Stalled) {
                            // Retain the connection until the CLI's real body timeout fires.
                            std::future::pending::<()>().await;
                        }
                        stream.shutdown().await.unwrap();
                    });
                }
                _ = connections.join_next(), if !connections.is_empty() => {}
            }
        }
    });
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join(".nyxid");
    std::fs::create_dir_all(&dir).unwrap();
    let token = format!(
        "e30.{}.sig",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&serde_json::json!({"exp": chrono::Utc::now().timestamp() + 3600}))
                .unwrap()
        )
    );
    for file in ["access_token", "refresh_token"] {
        std::fs::write(dir.join(file), &token).unwrap();
    }
    std::fs::write(dir.join("base_url"), &destination).unwrap();
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        tokio::process::Command::new(env!("CARGO_BIN_EXE_nyxid"))
            .args(["session", "status", "--output", "json"])
            .env("HOME", home.path())
            .env_remove("NYXID_URL")
            .env_remove("NYXID_PROFILE")
            .env_remove("NYXID_CA_CERT")
            .env_remove("SSL_CERT_FILE")
            .env_remove("SSL_CERT_DIR")
            .env("DO_NOT_TRACK", "1")
            .env("NYXID_NO_UPDATE_CHECK", "1")
            .env("NYXID_SKIP_SKILL_SELF_HEAL", "1")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("status must bound a stalled response body")
    .unwrap();
    server.abort();
    let _ = server.await;
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["server_check"], expected, "{report}");
    assert_eq!(report["http_status"], 200);
    assert_eq!(report["renewal"], "unknown");
    assert_eq!(report["refresh_usability_tested"], false);
    assert_eq!(
        *requests.lock().unwrap(),
        ["GET /api/v1/sessions HTTP/1.1"],
        "status must make exactly one lookup and zero refresh requests"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("refresh_token")).unwrap(),
        token
    );
}

#[tokio::test]
async fn status_stalled_body_is_unavailable_without_refreshing() {
    check_body_failure(Body::Stalled, "unavailable").await;
}

#[tokio::test]
async fn status_truncated_body_is_unavailable_without_refreshing() {
    check_body_failure(Body::Truncated, "unavailable").await;
}

#[tokio::test]
async fn status_complete_invalid_json_is_invalid_response_without_refreshing() {
    check_body_failure(Body::InvalidJson, "invalid_response").await;
}
