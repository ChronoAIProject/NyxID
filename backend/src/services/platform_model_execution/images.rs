//! Inline raster validation. Never resolves a URL or retains decoded pixels.
use super::adapters::unsupported;
use crate::errors::{AppError, AppResult};
use base64::{Engine, engine::general_purpose::STANDARD};
use image::{ImageDecoder, ImageFormat, ImageReader};
use serde_json::Value;
use std::{
    collections::HashSet,
    io::Cursor,
    sync::{Arc, LazyLock, Mutex},
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
const DECODE_BUDGET: usize = 384 * 1024 * 1024;
const PERSON_DECODE_BUDGET: usize = DECODE_BUDGET / 2;
static MEMORY: LazyLock<Arc<DecodeBudget>> = LazyLock::new(|| Arc::new(DecodeBudget::new()));

pub(crate) fn contains_images(body: &Value) -> bool {
    body["messages"].as_array().is_some_and(|messages| {
        messages.iter().any(|m| {
            m["content"]
                .as_array()
                .is_some_and(|parts| parts.iter().any(|p| p["type"] == "image_url"))
        })
    })
}
pub(super) struct DecodeBudget {
    memory: Arc<Semaphore>,
    people: Mutex<HashSet<String>>,
}

impl DecodeBudget {
    pub(super) fn new() -> Self {
        Self {
            memory: Arc::new(Semaphore::new(DECODE_BUDGET)),
            people: Mutex::new(HashSet::new()),
        }
    }

    pub(super) fn reserve(self: &Arc<Self>, person: &str, size: usize) -> AppResult<DecodePermit> {
        if size > MAX_REQUEST {
            return Err(unsupported());
        }
        // Validation copies are temporary and precede raster decoding. During
        // decode retain the ingress bytes/JSON, one decoded file and one pixel
        // buffer; translated requests and dispatch are outside this lifetime.
        let weight = size
            .checked_mul(2)
            .and_then(|n| n.checked_add(MAX_DECODE as usize + MAX_FILE))
            .ok_or_else(unsupported)?;
        let mut people = self.people.lock().unwrap_or_else(|e| e.into_inner());
        if weight > PERSON_DECODE_BUDGET || people.contains(person) {
            return Err(AppError::ServiceConcurrencyLimited);
        }
        let memory = self
            .memory
            .clone()
            .try_acquire_many_owned(weight as u32)
            .map_err(|_| AppError::ServiceConcurrencyLimited)?;
        // Only active decodes enter this set, so its cardinality is bounded by
        // the global budget. Refused callers never leave an identity behind.
        people.insert(person.to_owned());
        Ok(DecodePermit {
            budget: self.clone(),
            person: person.to_owned(),
            memory: Some(memory),
        })
    }
}

pub(super) struct DecodePermit {
    budget: Arc<DecodeBudget>,
    person: String,
    memory: Option<OwnedSemaphorePermit>,
}

impl Drop for DecodePermit {
    fn drop(&mut self) {
        let mut people = self.budget.people.lock().unwrap_or_else(|e| e.into_inner());
        drop(self.memory.take());
        people.remove(&self.person);
    }
}

pub(super) fn decode_budget() -> Arc<DecodeBudget> {
    MEMORY.clone()
}

#[cfg(test)]
pub(crate) fn hold_decode_for_test(person: &str) -> impl Send {
    decode_budget().reserve(person, 0).unwrap()
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

#[cfg(test)]
mod decode_budget_tests {
    use super::*;

    #[test]
    fn one_person_cannot_exhaust_capacity_or_leave_refused_identities() {
        let budget = Arc::new(DecodeBudget::new());
        // Even two requests at the legal total-file and non-image ceilings fit
        // together. Base64 padding/URI overhead is included conservatively.
        let size = MAX_FILES.div_ceil(3) * 4 + MAX_IMAGES * 128 + 1024 * 1024;
        let a = budget.reserve("person-a", size).unwrap();
        for _ in 0..100 {
            assert!(matches!(
                budget.reserve("person-a", 0),
                Err(AppError::ServiceConcurrencyLimited)
            ));
        }
        let b = budget.reserve("person-b", size).unwrap();
        assert!(matches!(
            budget.reserve("person-c", 0),
            Err(AppError::ServiceConcurrencyLimited)
        ));
        assert_eq!(budget.people.lock().unwrap().len(), 2);
        drop(a);
        let c = budget.reserve("person-c", 0).unwrap();
        drop((b, c));
        assert_eq!(budget.memory.available_permits(), DECODE_BUDGET);
        assert!(budget.people.lock().unwrap().is_empty());
    }

    #[test]
    fn per_person_weight_and_invalid_size_fail_without_reserving() {
        let budget = Arc::new(DecodeBudget::new());
        assert!(matches!(
            budget.reserve("person", MAX_REQUEST),
            Err(AppError::ServiceConcurrencyLimited)
        ));
        assert!(budget.reserve("person", MAX_REQUEST + 1).is_err());
        assert_eq!(budget.memory.available_permits(), DECODE_BUDGET);
        assert!(budget.people.lock().unwrap().is_empty());
    }
}
