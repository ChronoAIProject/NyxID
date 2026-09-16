//! Ephemeral media materialization and bounded, SSRF-safe downloads (ADR-013).
use std::time::Duration;

use base64::Engine;
use bytes::{Bytes, BytesMut};
use futures::StreamExt;
use serde_json::Value;

use super::channel_platform::{
    FetchedMedia, InboundAttachment, MaterializedAttachment, OutboundAttachment,
    OutboundMediaSource,
};
use crate::errors::{AppError, AppResult};

const TIMEOUT: Duration = Duration::from_secs(30);

pub fn fetch_failed() -> AppError {
    AppError::ChannelMediaFetchFailed(
        "Media could not be retrieved from the approved provider".into(),
    )
}
pub fn upload_failed() -> AppError {
    AppError::ChannelPlatformError("Media upload or delivery failed".into())
}

/// Match DNS labels, never substrings, userinfo, alternate ports or redirects.
pub fn allowed_host(url: &url::Url, hosts: &[&str]) -> bool {
    url.host_str().is_some_and(|host| {
        hosts.iter().any(|allowed| {
            if let Some(suffix) = allowed.strip_prefix("*.") {
                host.ends_with(&format!(".{suffix}")) && host.len() > suffix.len() + 1
            } else {
                host.eq_ignore_ascii_case(allowed)
            }
        })
    })
}

/// Test overrides are adapter-owned bases, never request-provided allowlist entries.
fn is_test_origin(url: &url::Url, trusted_base: Option<&str>) -> bool {
    #[cfg(test)]
    if let Some(base) = trusted_base.and_then(|b| url::Url::parse(b).ok()) {
        return base.host_str() == Some("127.0.0.1") && url.origin() == base.origin();
    }
    let _ = (url, trusted_base);
    false
}

pub async fn media_client(
    target: &str,
    hosts: Option<&[&str]>,
    trusted_base: Option<&str>,
) -> AppResult<reqwest::Client> {
    tokio::time::timeout(TIMEOUT, async {
        let parsed = url::Url::parse(target).map_err(|_| fetch_failed())?;
        if is_test_origin(&parsed, trusted_base) {
            return reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(TIMEOUT)
                .build()
                .map_err(|_| fetch_failed());
        }
        if parsed.scheme() != "https"
            || (hosts.is_some() && parsed.port_or_known_default() != Some(443))
            || hosts.is_some_and(|hosts| !allowed_host(&parsed, hosts))
        {
            return Err(fetch_failed());
        }
        super::webhook_delivery_service::validate_webhook_url(target, "Media URL")
            .await
            .map_err(|_| fetch_failed())?;
        let host = parsed.host_str().ok_or_else(fetch_failed)?;
        // Pin the addresses used for this request and re-check after resolution;
        // no DNS rebinding or redirect may bypass the public-host boundary.
        let addresses: Vec<_> = tokio::net::lookup_host((
            host,
            parsed.port_or_known_default().ok_or_else(fetch_failed)?,
        ))
        .await
        .map_err(|_| fetch_failed())?
        .collect();
        if addresses.is_empty()
            || addresses
                .iter()
                .any(|addr| super::url_validation::is_private_or_internal_ip(addr.ip()))
        {
            return Err(fetch_failed());
        }
        reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(TIMEOUT)
            .resolve_to_addrs(host, &addresses)
            .build()
            .map_err(|_| fetch_failed())
    })
    .await
    .map_err(|_| fetch_failed())?
}

pub async fn bounded_response(
    response: reqwest::Response,
    max_bytes: u64,
) -> AppResult<FetchedMedia> {
    if !response.status().is_success() {
        return Err(fetch_failed());
    }
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes)
    {
        return Err(AppError::ChannelMediaTooLarge);
    }
    let mime_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let mut bytes = BytesMut::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| fetch_failed())?;
        if (bytes.len() as u64).saturating_add(chunk.len() as u64) > max_bytes {
            return Err(AppError::ChannelMediaTooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(FetchedMedia {
        bytes: bytes.freeze(),
        mime_type,
        filename: None,
    })
}

pub async fn download(
    target: &str,
    hosts: &[&str],
    trusted_base: Option<&str>,
    bearer: Option<&str>,
    attachment: &InboundAttachment,
    max_bytes: u64,
) -> AppResult<FetchedMedia> {
    let client = media_client(target, Some(hosts), trusted_base).await?;
    if attachment.size_bytes.is_some_and(|size| size > max_bytes) {
        return Err(AppError::ChannelMediaTooLarge);
    }
    let mut request = client.get(target);
    if let Some(token) = bearer {
        request = request.bearer_auth(token);
    }
    let response = request.send().await.map_err(|_| fetch_failed())?;
    let mut media = bounded_response(response, max_bytes).await?;
    media.filename = attachment.filename.clone();
    media.mime_type = media.mime_type.or_else(|| attachment.mime_type.clone());
    Ok(media)
}

pub fn decode_base64(data: &str, max_bytes: u64) -> AppResult<Bytes> {
    // STANDARD requires padding. Compute the actual decoded length before
    // allocating so a nearly full final quartet cannot exceed the byte cap.
    let padding = data
        .as_bytes()
        .iter()
        .rev()
        .take_while(|&&b| b == b'=')
        .count()
        .min(2);
    let decoded_len = (data.len() as u64 / 4)
        .saturating_mul(3)
        .saturating_sub(padding as u64);
    if decoded_len > max_bytes || data.len() as u64 > max_bytes.saturating_add(2) / 3 * 4 {
        return Err(AppError::ChannelMediaTooLarge);
    }
    let mut bytes =
        vec![0; usize::try_from(decoded_len).map_err(|_| AppError::ChannelMediaTooLarge)?];
    let length = base64::engine::general_purpose::STANDARD
        .decode_slice(data, &mut bytes)
        .map_err(|_| AppError::ValidationError("Attachment data must be valid base64".into()))?;
    bytes.truncate(length);
    Ok(Bytes::from(bytes))
}

pub async fn materialize(
    attachments: &[OutboundAttachment],
    max_bytes: u64,
) -> AppResult<Vec<MaterializedAttachment>> {
    let mut result = Vec::with_capacity(attachments.len());
    for attachment in attachments {
        let media = match &attachment.source {
            OutboundMediaSource::Base64 { data } => FetchedMedia {
                bytes: decode_base64(data, max_bytes)?,
                mime_type: None,
                filename: None,
            },
            OutboundMediaSource::Url { url } => {
                let client = media_client(url, None, None).await?;
                let response = client.get(url).send().await.map_err(|_| fetch_failed())?;
                bounded_response(response, max_bytes).await?
            }
        };
        result.push(MaterializedAttachment {
            kind: attachment.kind,
            bytes: media.bytes,
            filename: attachment.filename.clone(),
            mime_type: attachment.mime_type.clone().or(media.mime_type),
            caption: attachment.caption.clone(),
        });
    }
    Ok(result)
}

/// Provider control JSON responses are bounded to 256 KiB, allowing attachment-rich objects.
pub async fn response_json(request: reqwest::RequestBuilder) -> AppResult<Value> {
    let response = request
        .timeout(TIMEOUT)
        .send()
        .await
        .map_err(|_| upload_failed())?;
    let media = bounded_response(response, 256 * 1024)
        .await
        .map_err(|_| upload_failed())?;
    serde_json::from_slice(&media.bytes).map_err(|_| upload_failed())
}

pub fn multipart_part(attachment: &MaterializedAttachment) -> AppResult<reqwest::multipart::Part> {
    let part = reqwest::multipart::Part::stream_with_length(
        reqwest::Body::from(attachment.bytes.clone()),
        attachment.bytes.len() as u64,
    )
    .file_name(safe_filename(attachment.filename.as_deref()));
    part.mime_str(
        attachment
            .mime_type
            .as_deref()
            .unwrap_or("application/octet-stream"),
    )
    .map_err(|_| AppError::ValidationError("Invalid attachment MIME type".into()))
}

pub fn safe_filename(filename: Option<&str>) -> String {
    let name: String = filename
        .unwrap_or("attachment")
        .chars()
        .take(180)
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if name.trim_matches([' ', '.']).is_empty() {
        "attachment".into()
    } else {
        name
    }
}

pub fn download_url(base_url: &str, message_id: &str, index: usize) -> String {
    format!(
        "{}/api/v1/channel-relay/messages/{}/attachments/{index}",
        base_url.trim_end_matches('/'),
        urlencoding::encode(message_id)
    )
}

pub fn request_body_limit(max_bytes: u64) -> usize {
    usize::try_from(
        max_bytes
            .saturating_add(2)
            .saturating_div(3)
            .saturating_mul(4)
            .saturating_add(64 * 1024),
    )
    .unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    #[tokio::test]
    async fn channel_media_control_json_allows_large_attachment_metadata_but_stays_bounded() {
        let server = MockServer::start().await;
        for size in [200 * 1024, 257 * 1024] {
            Mock::given(method("GET"))
                .and(path(format!("/{size}")))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "metadata": "x".repeat(size)
                })))
                .expect(1)
                .mount(&server)
                .await;
            let result =
                response_json(reqwest::Client::new().get(format!("{}/{size}", server.uri()))).await;
            if size < 256 * 1024 {
                assert_eq!(result.unwrap()["metadata"].as_str().unwrap().len(), size);
            } else {
                assert!(matches!(result, Err(AppError::ChannelPlatformError(_))));
            }
        }
    }

    #[tokio::test]
    async fn channel_media_bounded_download_and_host_fences() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/large"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![1; 100]))
            .mount(&server)
            .await;
        let response = reqwest::Client::new()
            .get(format!("{}/large", server.uri()))
            .send()
            .await
            .unwrap();
        assert!(matches!(
            bounded_response(response, 4).await,
            Err(AppError::ChannelMediaTooLarge)
        ));
        for url in [
            "http://files.slack.com/a",
            "https://files.slack.com.evil.test/a",
            "https://127.0.0.1/a",
            "https://user:secret@files.slack.com/a",
            "https://files.slack.com:8443/a",
        ] {
            let error = media_client(url, Some(&["files.slack.com", "*.slack.com"]), None)
                .await
                .unwrap_err();
            assert!(matches!(error, AppError::ChannelMediaFetchFailed(_)));
            assert!(!error.to_string().contains(url));
        }
        assert!(allowed_host(
            &url::Url::parse("https://a.slack.com/a").unwrap(),
            &["*.slack.com"]
        ));
        assert!(!allowed_host(
            &url::Url::parse("https://evilslack.com/a").unwrap(),
            &["*.slack.com"]
        ));
    }

    #[tokio::test]
    async fn channel_media_chunked_overflow_stops_before_copying_large_chunk() {
        use axum::{Router, body::Body, routing::get};
        let app = Router::new().route(
            "/stream",
            get(|| async {
                Body::from_stream(futures::stream::iter([
                    Ok::<_, std::io::Error>(Bytes::from_static(b"1234")),
                    Ok(Bytes::from_static(b"5678")),
                ]))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let response = reqwest::Client::new()
            .get(format!("http://{address}/stream"))
            .send()
            .await
            .unwrap();
        assert!(response.content_length().is_none());
        assert!(matches!(
            bounded_response(response, 4).await,
            Err(AppError::ChannelMediaTooLarge)
        ));
        task.abort();
    }

    #[tokio::test]
    async fn channel_media_url_sources_reject_private_hosts_before_fetching() {
        use super::super::channel_platform::MediaKind;
        for url in [
            "http://127.0.0.1/private",
            "https://127.0.0.1/private",
            "file:///etc/passwd",
            "https://[::1]/private",
        ] {
            let attachment = OutboundAttachment {
                kind: MediaKind::File,
                source: OutboundMediaSource::Url { url: url.into() },
                filename: None,
                mime_type: None,
                caption: None,
            };
            assert!(matches!(
                materialize(&[attachment], 100).await,
                Err(AppError::ChannelMediaFetchFailed(_))
            ));
        }
    }

    #[test]
    fn channel_media_base64_cap_and_redaction() {
        assert_eq!(decode_base64("aGVsbG8=", 5).unwrap(), b"hello"[..]);
        assert!(matches!(
            decode_base64("aGVsbG8=", 4),
            Err(AppError::ChannelMediaTooLarge)
        ));
        assert!(matches!(
            decode_base64("!!!", 5),
            Err(AppError::ValidationError(_))
        ));
        let media = FetchedMedia {
            bytes: Bytes::from_static(b"secret"),
            filename: Some("private.pdf".into()),
            mime_type: None,
        };
        assert_eq!(format!("{media:?}"), "FetchedMedia([REDACTED])");
        assert_eq!(safe_filename(Some("../a\r\n\".pdf")), ".._a___.pdf");
    }
}
