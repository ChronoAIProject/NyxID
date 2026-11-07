//! Inline raster validation. Never resolves a URL or retains decoded pixels.
use super::adapters::unsupported;
use crate::errors::{AppError, AppResult};
use base64::{Engine, engine::general_purpose::STANDARD};
use image::{ImageDecoder, ImageFormat, ImageReader};
use serde_json::Value;
use std::{
    io::Cursor,
    sync::{Arc, LazyLock},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub(crate) const READINESS: &str = "inference:platform-model-images-v1";
pub(crate) const MAX_REQUEST: usize = 32 * 1024 * 1024;
pub(crate) const MAX_FILE: usize = 8 * 1024 * 1024;
pub(crate) const MAX_FILES: usize = 16 * 1024 * 1024;
pub(crate) const MAX_IMAGES: usize = 10;
const MAX_PIXELS: u64 = 32_000_000;
const MAX_TOTAL_PIXELS: u64 = 64_000_000;
const MAX_DECODE: u64 = 128 * 1024 * 1024;
// Same weighted-permit pattern as attachment byte_budget. Account for original
// bytes, JSON, validation serialization, translated JSON and HTTP bytes as well
// as one sequential decoder. No semaphore wait may accumulate decoded images.
static MEMORY: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(384 * 1024 * 1024)));

pub(super) fn contains_images(body: &Value) -> bool {
    body["messages"].as_array().is_some_and(|messages| {
        messages.iter().any(|m| {
            m["content"]
                .as_array()
                .is_some_and(|parts| parts.iter().any(|p| p["type"] == "image_url"))
        })
    })
}
pub(super) fn reserve(size: usize) -> AppResult<Arc<OwnedSemaphorePermit>> {
    if size > MAX_REQUEST {
        return Err(unsupported());
    }
    let weight = size
        .checked_mul(5)
        .and_then(|n| n.checked_add(MAX_DECODE as usize + MAX_FILE))
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(unsupported)?;
    MEMORY
        .clone()
        .try_acquire_many_owned(weight)
        .map(Arc::new)
        .map_err(|_| AppError::ServiceConcurrencyLimited)
}

#[derive(Clone, Default)]
pub(super) struct Summary {
    pub count: usize,
    pub bytes: usize,
    pub largest_file: usize,
    pub largest_edge: u32,
    pub pixels: u64,
    pub uri_bytes: usize,
}

pub(super) fn data_uri(part: &Value) -> AppResult<(&str, &str)> {
    let image = part["image_url"].as_object().ok_or_else(unsupported)?;
    if image
        .keys()
        .any(|k| !matches!(k.as_str(), "url" | "detail"))
        || image.get("detail").is_some_and(|v| v != "auto")
    {
        return Err(unsupported());
    }
    let uri = image
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(unsupported)?;
    let (mime, data) = uri
        .strip_prefix("data:")
        .and_then(|s| s.split_once(";base64,"))
        .ok_or_else(unsupported)?;
    if !matches!(mime, "image/png" | "image/jpeg" | "image/webp")
        || data.is_empty()
        || data.len() > MAX_FILE.div_ceil(3) * 4
    {
        return Err(unsupported());
    }
    Ok((mime, data))
}

/// Walk container chunks before decoder allocation: reject animation, trailing
/// concatenated containers and malformed lengths, including APNG default frames.
fn static_container(bytes: &[u8], format: ImageFormat) -> AppResult<()> {
    match format {
        ImageFormat::Png => {
            let mut at = 8_usize;
            let mut end = false;
            while at < bytes.len() {
                let header = bytes.get(at..at + 8).ok_or_else(unsupported)?;
                let size = u32::from_be_bytes(header[..4].try_into().unwrap()) as usize;
                let kind = &header[4..];
                if matches!(kind, b"acTL" | b"fcTL" | b"fdAT") {
                    return Err(unsupported());
                }
                at = at
                    .checked_add(12)
                    .and_then(|n| n.checked_add(size))
                    .filter(|n| *n <= bytes.len())
                    .ok_or_else(unsupported)?;
                if kind == b"IEND" {
                    end = size == 0 && at == bytes.len();
                    break;
                }
            }
            if !end {
                return Err(unsupported());
            }
        }
        ImageFormat::WebP => {
            let size = bytes.get(4..8).ok_or_else(unsupported)?;
            if u32::from_le_bytes(size.try_into().unwrap()) as usize + 8 != bytes.len() {
                return Err(unsupported());
            }
            let mut at = 12_usize;
            while at < bytes.len() {
                let header = bytes.get(at..at + 8).ok_or_else(unsupported)?;
                let size = u32::from_le_bytes(header[4..].try_into().unwrap()) as usize;
                if matches!(&header[..4], b"ANIM" | b"ANMF")
                    || &header[..4] == b"VP8X"
                        && bytes.get(at + 8).is_none_or(|flags| flags & 2 != 0)
                {
                    return Err(unsupported());
                }
                at = at
                    .checked_add(8)
                    .and_then(|n| n.checked_add(size))
                    .and_then(|n| n.checked_add(size % 2))
                    .filter(|n| *n <= bytes.len())
                    .ok_or_else(unsupported)?;
            }
        }
        ImageFormat::Jpeg => {
            // Walk marker lengths, not arbitrary byte matches in metadata or
            // compressed scans. MPO/multiple images must never become frame 1.
            let mut at = 2_usize;
            loop {
                if bytes.get(at) != Some(&0xff) {
                    return Err(unsupported());
                }
                while bytes.get(at) == Some(&0xff) {
                    at += 1;
                }
                let marker = *bytes.get(at).ok_or_else(unsupported)?;
                at += 1;
                if marker == 0xd9 {
                    if at != bytes.len() {
                        return Err(unsupported());
                    }
                    break;
                }
                if marker == 0xd8 || marker == 0 || (0xd0..=0xd7).contains(&marker) {
                    return Err(unsupported());
                }
                let length = bytes.get(at..at + 2).ok_or_else(unsupported)?;
                let length = u16::from_be_bytes(length.try_into().unwrap()) as usize;
                if length < 2 {
                    return Err(unsupported());
                }
                let end = at
                    .checked_add(length)
                    .filter(|n| *n <= bytes.len())
                    .ok_or_else(unsupported)?;
                if marker == 0xe2 && bytes[at + 2..end].starts_with(b"MPF\0") {
                    return Err(unsupported());
                }
                at = end;
                if marker == 0xda {
                    loop {
                        let byte = *bytes.get(at).ok_or_else(unsupported)?;
                        if byte == 0xff {
                            let next = *bytes.get(at + 1).ok_or_else(unsupported)?;
                            if next == 0 || (0xd0..=0xd7).contains(&next) {
                                at += 2;
                                continue;
                            }
                            break;
                        }
                        at += 1;
                    }
                }
            }
        }
        _ => return Err(unsupported()),
    }
    Ok(())
}

impl Summary {
    pub fn push(&mut self, part: &Value) -> AppResult<()> {
        if self.count >= MAX_IMAGES {
            return Err(unsupported());
        }
        let (mime, data) = data_uri(part)?;
        let bytes = STANDARD.decode(data).map_err(|_| unsupported())?;
        if bytes.len() > MAX_FILE || self.bytes + bytes.len() > MAX_FILES {
            return Err(unsupported());
        }
        let format = image::guess_format(&bytes).map_err(|_| unsupported())?;
        if !matches!(
            (mime, format),
            ("image/png", ImageFormat::Png)
                | ("image/jpeg", ImageFormat::Jpeg)
                | ("image/webp", ImageFormat::WebP)
        ) {
            return Err(unsupported());
        }
        static_container(&bytes, format)?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(MAX_PIXELS as u32);
        limits.max_image_height = Some(MAX_PIXELS as u32);
        limits.max_alloc = Some(MAX_DECODE);
        let mut reader = ImageReader::with_format(Cursor::new(&bytes), format);
        reader.limits(limits);
        let decoder = reader.into_decoder().map_err(|_| unsupported())?;
        let (width, height) = decoder.dimensions();
        let pixels = u64::from(width) * u64::from(height);
        if pixels == 0
            || pixels > MAX_PIXELS
            || self.pixels + pixels > MAX_TOTAL_PIXELS
            || decoder.total_bytes() > MAX_DECODE
        {
            return Err(unsupported());
        }
        // Full bounded decode rejects truncated/corrupt payloads. Pixels are
        // discarded immediately; the original raster alone is sent upstream.
        let mut decoded =
            vec![0; usize::try_from(decoder.total_bytes()).map_err(|_| unsupported())?];
        decoder
            .read_image(&mut decoded)
            .map_err(|_| unsupported())?;
        self.count += 1;
        self.bytes += bytes.len();
        self.largest_file = self.largest_file.max(bytes.len());
        self.largest_edge = self.largest_edge.max(width).max(height);
        self.pixels += pixels;
        self.uri_bytes += part["image_url"]["url"]
            .as_str()
            .ok_or_else(unsupported)?
            .len();
        Ok(())
    }
}

/// Count serialized bytes without allocating another base64-sized JSON buffer.
pub(super) fn json_size(body: &Value) -> AppResult<usize> {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_add(b.len())
                .filter(|n| *n <= MAX_REQUEST)
                .ok_or_else(|| std::io::Error::other("request bound"))?;
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, body).map_err(|_| unsupported())?;
    Ok(counter.0)
}

/// Computed public metadata only; never persisted in a capability/tariff row.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct PublicLimits {
    pub accounting: &'static str,
    pub mime_types: [&'static str; 3],
    pub max_images: usize,
    pub max_file_bytes: usize,
    pub max_total_file_bytes: usize,
    pub max_image_pixels: u64,
    pub max_total_pixels: u64,
    pub max_request_bytes: usize,
    pub detail: &'static str,
}
pub(crate) fn public_limits(
    model: &crate::models::platform_model::PlatformModel,
    routes: &[crate::models::platform_model::ModelRoute],
    tariff: &crate::models::platform_model_tariff::ModelTariff,
) -> Option<PublicLimits> {
    use super::adapters::Native;
    use crate::models::platform_model::{Modality, ModelFamily};
    if !model
        .capabilities
        .input_modalities
        .contains(&Modality::Image)
        || !model
            .capabilities
            .protocols
            .contains(&crate::models::platform_model::CallerProtocol::OpenaiChat)
        || model
            .capabilities
            .context_tokens
            .is_none_or(|n| n == 0 || i64::from(n) > tariff.bounds.max_input_tokens)
        || model.capabilities.output_tokens.is_none_or(|n| n == 0)
    {
        return None;
    }
    let native = match model.family {
        ModelFamily::Gemini => Native::Gemini,
        ModelFamily::Claude => Native::Claude,
        ModelFamily::Openai => Native::Responses,
        ModelFamily::OpenModel => return None,
    };
    if !routes.iter().any(|r| {
        r.enabled
            && crate::services::platform_model_service::validate_route(model, r).is_ok()
            && super::certification::certify(native, &r.target, tariff, true).is_ok()
    }) {
        return None;
    }
    Some(PublicLimits {
        accounting: if tariff.contract.input_modalities.is_empty() {
            "included_in_input_tokens"
        } else {
            "exact_image_and_cache_partition"
        },
        mime_types: ["image/png", "image/jpeg", "image/webp"],
        max_images: MAX_IMAGES.min(usize::try_from(tariff.bounds.max_images).ok()?),
        max_file_bytes: if native == Native::Claude {
            5 * 1024 * 1024
        } else {
            MAX_FILE
        },
        max_total_file_bytes: MAX_FILES,
        max_image_pixels: MAX_PIXELS,
        max_total_pixels: MAX_TOTAL_PIXELS,
        max_request_bytes: MAX_REQUEST.min(usize::try_from(tariff.bounds.max_request_bytes).ok()?),
        detail: "auto",
    })
}
