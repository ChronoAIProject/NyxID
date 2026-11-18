use super::*;
use crate::services::{
    attachment_extraction, channel_media_service as media, content_type,
    lark_attachment_tool::multipart, mcp_service,
};
use std::time::Duration;

pub const MAX_BYTES: usize = 10 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);

fn invalid() -> AppError {
    AppError::ValidationError("lark_image_input_invalid: supply exactly one image_url or image_base64, and an optional filename and matching image mime_type".into())
}

pub fn validate(body: &Value) -> AppResult<()> {
    let object = body.as_object().ok_or_else(invalid)?;
    if object.keys().any(|key| {
        !matches!(
            key.as_str(),
            "image_url" | "image_base64" | "filename" | "mime_type"
        )
    }) || object.contains_key("image_url") == object.contains_key("image_base64")
    {
        return Err(invalid());
    }
    for key in ["image_url", "image_base64", "filename", "mime_type"] {
        if let Some(value) = object.get(key) {
            let value = value
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(invalid)?;
            let max = match key {
                "image_base64" => MAX_BYTES.div_ceil(3) * 4,
                "image_url" => media::MAX_MEDIA_URL_BYTES,
                "filename" => 255,
                _ => 128,
            };
            if value.len() > max {
                return Err(invalid());
            }
        }
    }
    if let Some(name) = body["filename"].as_str()
        && (name.chars().any(char::is_control)
            || name.contains(['/', '\\'])
            || matches!(name.trim(), "" | "." | ".."))
    {
        return Err(invalid());
    }
    if let Some(mime) = body["mime_type"].as_str()
        && !matches!(
            mime,
            "image/png" | "image/jpeg" | "image/gif" | "image/webp"
        )
    {
        return Err(invalid());
    }
    Ok(())
}

/// Run only after the normal MCP scope, guest and confirmation gates. Fetch
/// uses a fresh credential-free public-IP-pinned client, with zero redirects.
pub async fn materialize(body: &[u8]) -> AppResult<(bytes::Bytes, String)> {
    materialize_with(body, None, TIMEOUT).await
}

async fn materialize_with(
    body: &[u8],
    trusted_test_origin: Option<&str>,
    timeout: Duration,
) -> AppResult<(bytes::Bytes, String)> {
    let args: Value = serde_json::from_slice(body).map_err(|_| invalid())?;
    validate(&args)?;
    tokio::time::timeout(timeout, async {
        let (bytes, response_mime) = if let Some(base64) = args["image_base64"].as_str() {
            (media::decode_base64(base64, MAX_BYTES as u64)?, None)
        } else {
            let url = args["image_url"].as_str().ok_or_else(invalid)?;
            let client = media::media_client(url, None, trusted_test_origin).await?;
            let response = client
                .get(url)
                .send()
                .await
                .map_err(|_| media::fetch_failed())?;
            let fetched = media::bounded_response(response, MAX_BYTES as u64).await?;
            // URL sources must declare their actual image type; no sniff-only
            // fallback for HTML, generic binary, or missing Content-Type.
            let mime = fetched.mime_type.ok_or_else(invalid)?;
            (fetched.bytes, Some(mime))
        };
        if bytes.is_empty() || bytes.len() > MAX_BYTES {
            return Err(AppError::ChannelMediaTooLarge);
        }
        let mime = ["image/png", "image/jpeg", "image/gif", "image/webp"]
            .into_iter()
            .find(|mime| mcp_service::image_magic_matches(mime, &bytes))
            .ok_or_else(invalid)?;
        if response_mime
            .as_deref()
            .is_some_and(|m| content_type::normalize_content_type(m) != mime)
            || args["mime_type"].as_str().is_some_and(|m| m != mime)
        {
            return Err(invalid());
        }
        let extracted = attachment_extraction::extract(bytes.to_vec(), String::new())
            .await
            .map_err(|_| invalid())?;
        if extracted.content_type != mime {
            return Err(invalid());
        }
        let default_name = match mime {
            "image/png" => "image.png",
            "image/jpeg" => "image.jpg",
            "image/gif" => "image.gif",
            _ => "image.webp",
        };
        multipart::encode_image(
            args["filename"].as_str().unwrap_or(default_name),
            mime,
            &bytes,
        )
    })
    .await
    .map_err(|_| {
        AppError::ChannelMediaFetchFailed("lark_image_fetch_timeout: image was not uploaded".into())
    })?
}

#[cfg(test)]
mod tests;
