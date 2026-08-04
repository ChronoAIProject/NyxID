use anyhow::{Context, Result, bail};
use tokio::io::AsyncWriteExt;

use crate::api::build_cli_http_client;
use crate::cli::PublicCommands;

pub async fn run(command: PublicCommands) -> Result<()> {
    match command {
        PublicCommands::Request {
            slug,
            path,
            method,
            data,
            headers,
            stream,
            base,
        } => {
            let base_url = base.resolved_base_url()?;
            let client = build_cli_http_client(base.profile.as_deref())?;
            let url = public_proxy_url(&base_url, &slug, &path);
            let method = reqwest::Method::from_bytes(method.to_ascii_uppercase().as_bytes())
                .with_context(|| format!("Invalid HTTP method: {method}"))?;

            let mut request = client.request(method, &url);
            for (name, value) in parse_headers(&headers)? {
                if name.eq_ignore_ascii_case("authorization")
                    || name.eq_ignore_ascii_case("cookie")
                    || name.to_ascii_lowercase().starts_with("x-nyxid-")
                {
                    continue;
                }
                request = request.header(name, value);
            }

            let body = read_body(data.as_deref())?;
            if let Some(body) = body {
                request = request.body(body);
            }

            let response = request
                .send()
                .await
                .with_context(|| format!("Public request to {url} failed"))?;
            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                bail!("Public request failed (HTTP {status}): {body}");
            }

            if stream {
                let mut stdout = tokio::io::stdout();
                let mut byte_stream = response.bytes_stream();
                use futures::StreamExt;
                while let Some(chunk) = byte_stream.next().await {
                    let bytes = chunk.context("Failed to read response chunk")?;
                    stdout.write_all(&bytes).await?;
                }
                stdout.flush().await?;
            } else {
                let body = response
                    .bytes()
                    .await
                    .context("Failed to read response body")?;
                let mut stdout = tokio::io::stdout();
                stdout.write_all(&body).await?;
                stdout.flush().await?;
            }

            Ok(())
        }
    }
}

fn public_proxy_url(base_url: &str, slug: &str, path: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    if path.is_empty() {
        format!("{base}/public/s/{slug}")
    } else {
        format!("{base}/public/s/{slug}/{path}")
    }
}

fn parse_headers(headers: &[String]) -> Result<Vec<(String, String)>> {
    headers
        .iter()
        .map(|header| {
            let Some((name, value)) = header.split_once(':') else {
                bail!("Header must be in Name: value form: {header}");
            };
            let name = name.trim();
            if name.is_empty() {
                bail!("Header name must not be empty");
            }
            Ok((name.to_string(), value.trim().to_string()))
        })
        .collect()
}

fn read_body(data: Option<&str>) -> Result<Option<Vec<u8>>> {
    match data {
        Some("-") => {
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut std::io::stdin(), &mut buf)
                .context("Failed to read stdin")?;
            Ok(Some(buf))
        }
        Some(value) if value.starts_with('@') => {
            let path = &value[1..];
            Ok(Some(
                std::fs::read(path).with_context(|| format!("Failed to read file: {path}"))?,
            ))
        }
        Some(value) => Ok(Some(value.as_bytes().to_vec())),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Commands};
    use clap::Parser;

    #[test]
    fn public_proxy_url_trims_slashes() {
        assert_eq!(
            public_proxy_url("https://auth.example/", "svc", "/public/a"),
            "https://auth.example/public/s/svc/public/a"
        );
        assert_eq!(
            public_proxy_url("https://auth.example", "svc", ""),
            "https://auth.example/public/s/svc"
        );
    }

    #[test]
    fn parse_public_request_command() {
        let cli = Cli::parse_from([
            "nyxid",
            "public",
            "request",
            "svc",
            "/public/a",
            "--base-url",
            "https://auth.example",
            "-H",
            "Accept: application/json",
        ]);

        match cli.command {
            Commands::Public { command } => match command {
                crate::cli::PublicCommands::Request {
                    slug,
                    path,
                    headers,
                    ..
                } => {
                    assert_eq!(slug, "svc");
                    assert_eq!(path, "/public/a");
                    assert_eq!(headers, vec!["Accept: application/json"]);
                }
            },
            _ => panic!("expected public command"),
        }
    }
}
