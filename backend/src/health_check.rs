//! Shell-free container liveness check. No application configuration or secrets.

use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(3);

fn port(value: Option<&str>) -> Result<u16, &'static str> {
    match value {
        None => Ok(3001),
        Some(value) if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) => {
            value.parse().map_err(|_| "invalid PORT")
        }
        Some(_) => Err("invalid PORT"),
    }
}

async fn check(port: u16, timeout: Duration) -> Result<(), &'static str> {
    // Explicit provider selection is independent of AppConfig and does not
    // read credentials. Reqwest constructs its TLS backend even for plain HTTP.
    crate::install_rustls_crypto_provider();
    let operation = async {
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(timeout)
            .build()
            .map_err(|_| "client initialization failed")?;
        let response = client
            .get(format!("http://127.0.0.1:{port}/health"))
            .send()
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    "timed out"
                } else {
                    "connection failed"
                }
            })?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err("non-success status")
        }
    };
    tokio::time::timeout(timeout, operation)
        .await
        .map_err(|_| "timed out")?
}

#[tokio::main(flavor = "current_thread")]
pub async fn run() -> i32 {
    let outcome = if std::env::args_os().count() != 2 {
        Err("invalid arguments")
    } else {
        let parsed = match std::env::var("PORT") {
            Ok(value) => port(Some(&value)),
            Err(std::env::VarError::NotPresent) => port(None),
            Err(std::env::VarError::NotUnicode(_)) => Err("invalid PORT"),
        };
        match parsed {
            Ok(port) => check(port, TIMEOUT).await,
            Err(reason) => Err(reason),
        }
    };
    if let Err(reason) = outcome {
        eprintln!("health-check: {reason}");
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn server(
        status: &str,
        delay: Duration,
        extra: &str,
    ) -> (u16, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let reply =
            format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n{extra}\r\n");
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let size = socket.read(&mut request).await.unwrap();
            assert!(
                std::str::from_utf8(&request[..size])
                    .unwrap()
                    .starts_with("GET /health HTTP/1.1\r\n")
            );
            tokio::time::sleep(delay).await;
            let _ = socket.write_all(reply.as_bytes()).await;
        });
        (port, handle)
    }

    #[test]
    fn health_check_validates_port_without_accepting_hosts_paths_or_whitespace() {
        assert_eq!(port(None), Ok(3001));
        for value in ["0", "1", "3001", "65535"] {
            assert_eq!(port(Some(value)), Ok(value.parse().unwrap()));
        }
        for value in [
            "",
            "65536",
            "-1",
            "+80",
            " 80",
            "80\n",
            "example.org",
            "80/health",
            "80@host",
        ] {
            assert_eq!(port(Some(value)), Err("invalid PORT"));
        }
    }

    #[tokio::test]
    async fn health_check_accepts_only_2xx() {
        for status in [
            "200 OK",
            "204 No Content",
            "299 Custom",
            "300 Multiple Choices",
            "401 Unauthorized",
            "503 Unavailable",
        ] {
            let (port, task) = server(status, Duration::ZERO, "").await;
            let result = check(port, TIMEOUT).await;
            assert_eq!(
                result,
                if status.starts_with('2') {
                    Ok(())
                } else {
                    Err("non-success status")
                }
            );
            task.await.unwrap();
        }
    }

    #[tokio::test]
    async fn health_check_has_a_total_timeout() {
        let (port, task) = server("200 OK", Duration::from_secs(5), "").await;
        assert_eq!(
            check(port, Duration::from_millis(40)).await,
            Err("timed out")
        );
        task.abort();
    }

    #[tokio::test]
    async fn health_check_connection_refused_is_a_stable_failure() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        assert_eq!(check(port, TIMEOUT).await, Err("connection failed"));
    }

    #[tokio::test]
    async fn health_check_never_follows_a_redirect() {
        let target = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let extra = format!(
            "Location: http://127.0.0.1:{}/health\r\n",
            target.local_addr().unwrap().port()
        );
        let (port, task) = server("302 Found", Duration::ZERO, &extra).await;
        assert_eq!(check(port, TIMEOUT).await, Err("non-success status"));
        assert!(
            tokio::time::timeout(Duration::from_millis(40), target.accept())
                .await
                .is_err()
        );
        task.await.unwrap();
    }
}
