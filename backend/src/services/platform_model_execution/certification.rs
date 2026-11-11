//! Immutable historical model mappings plus versioned family/transport contracts. Fixture digests are
//! immutable; new certification never rewrites a historical tariff's evidence.
use super::{
    adapters::{Native, unsupported},
    images,
};
use crate::{
    errors::AppResult,
    models::{
        platform_model::*,
        platform_model_tariff::{Modality as UsageModality, ModelTariff, UsageContract},
    },
};
use sha2::{Digest, Sha256};

pub(super) fn image_fixture(route: &RouteTarget) -> Option<&'static str> {
    match route {
        RouteTarget::VertexPublisher {
            publisher: VertexPublisher::Google,
            native_model,
            ..
        } if native_model == "gemini-2.5-flash" => {
            Some(include_str!("fixtures/gemini-images-v1.json"))
        }
        RouteTarget::VertexPublisher {
            publisher: VertexPublisher::Anthropic,
            native_model,
            ..
        } if native_model == "claude-sonnet-4-5" => {
            Some(include_str!("fixtures/claude-vertex-images-v1.json"))
        }
        RouteTarget::DirectProvider {
            provider: DirectProvider::Anthropic,
            native_model,
        } if native_model == "claude-sonnet-4-5" => {
            Some(include_str!("fixtures/claude-direct-images-v1.json"))
        }
        RouteTarget::DirectProvider {
            provider: DirectProvider::Openai,
            native_model,
        } if native_model == "gpt-5" => Some(include_str!("fixtures/responses-images-v1.json")),
        _ => None,
    }
}
pub(super) fn digest(fixture: &str) -> String {
    format!("{:x}", Sha256::digest(fixture.as_bytes()))
}
fn claude_image_digest() -> String {
    // One immutable evidence bundle certifies both same-model transports, so
    // a single tariff can safely fall back between Vertex and direct Claude.
    {
        let mut hash = Sha256::new();
        hash.update(include_bytes!("fixtures/claude-vertex-images-v1.json"));
        hash.update(include_bytes!("fixtures/claude-direct-images-v1.json"));
        format!("{:x}", hash.finalize())
    }
}
pub(super) fn image_digest(route: &RouteTarget) -> Option<String> {
    let fixture = image_fixture(route)?;
    Some(match route {
        RouteTarget::VertexPublisher {
            publisher: VertexPublisher::Anthropic,
            ..
        }
        | RouteTarget::DirectProvider {
            provider: DirectProvider::Anthropic,
            ..
        } => claude_image_digest(),
        _ => digest(fixture),
    })
}
pub(crate) fn image_contract(c: &UsageContract) -> bool {
    family_contract(c)
        || [
            digest(include_str!("fixtures/gemini-images-v1.json")),
            claude_image_digest(),
            digest(include_str!("fixtures/responses-images-v1.json")),
        ]
        .contains(&c.fixture_sha256)
}

pub(super) fn certify(
    native: Native,
    route: &RouteTarget,
    tariff: &ModelTariff,
    has_images: bool,
) -> AppResult<()> {
    let c = &tariff.contract;
    if family_contract(c) {
        return certify_family(native, route, tariff, has_images);
    }
    let exact = match route {
        RouteTarget::VertexPublisher {
            publisher: VertexPublisher::Google,
            native_model,
            ..
        } => native_model == "gemini-2.5-flash",
        RouteTarget::VertexPublisher {
            publisher: VertexPublisher::Anthropic,
            native_model,
            ..
        }
        | RouteTarget::DirectProvider {
            provider: DirectProvider::Anthropic,
            native_model,
        } => native_model == "claude-sonnet-4-5",
        RouteTarget::DirectProvider {
            provider: DirectProvider::Openai,
            native_model,
        } => native_model == "gpt-5",
        RouteTarget::VertexOpenapiMaas {
            publisher,
            native_model,
            ..
        } => publisher == "openai" && native_model == "gpt-oss-120b-maas",
    };
    if !exact || native.family() != c.family {
        return Err(unsupported());
    }
    if c.fixture_sha256 == native.digest() && !has_images {
        return Ok(());
    }
    let expected_digest = image_digest(route).ok_or_else(unsupported)?;
    let version = match native {
        Native::Gemini => "gemini.generate.v1",
        Native::Claude => "anthropic.messages.v1",
        Native::Responses => "openai.responses.v1",
        Native::GptOss => return Err(unsupported()),
    };
    // Both aggregate and exact Gemini partitions are finite certified contracts.
    // Responses/Claude do not claim a modality split from aggregate evidence.
    if c.fixture_sha256 != expected_digest
        || c.version != version
        || !c.tokens
        || !c.cache_read
        || c.images
        || c.bill_tool_input
        || !c.output_modalities.is_empty()
        || !(c.input_modalities.is_empty()
            || native == Native::Gemini && c.input_modalities == [UsageModality::Image])
        || native != Native::Claude && !c.cache_write_ttls.is_empty()
        || native == Native::Claude && c.split_reasoning
        || tariff.bounds.max_images == 0
    {
        return Err(unsupported());
    }
    Ok(())
}

pub(super) fn request_limits(native: Native, image: &images::Summary) -> AppResult<()> {
    // Anthropic's per-image native limit is smaller than the shared ceiling.
    if native == Native::Claude
        && (image.largest_file > 5 * 1024 * 1024 || image.largest_edge > 8000)
    {
        return Err(unsupported());
    }
    Ok(())
}

/// New digests are unreadable as executable certifications by old replicas.
pub(crate) const READINESS: &str = "inference:platform-model-contracts-v1";

pub(crate) fn family_fixture(
    family: crate::models::platform_model_tariff::UsageFamily,
) -> Option<&'static str> {
    use crate::models::platform_model_tariff::UsageFamily;
    match family {
        UsageFamily::Gemini => Some(include_str!("fixtures/gemini-contract-v2.json")),
        UsageFamily::Claude => Some(include_str!("fixtures/claude-contract-v2.json")),
        UsageFamily::Openai => Some(include_str!("fixtures/responses-contract-v2.json")),
        UsageFamily::OpenMaas => None,
    }
}

pub(crate) fn family_contract(c: &UsageContract) -> bool {
    family_fixture(c.family).is_some_and(|f| c.fixture_sha256 == digest(f))
}

pub(crate) fn validate_family_tariff(t: &ModelTariff) -> AppResult<()> {
    if !family_contract(&t.contract) {
        return if t.contract.version.ends_with(".v2") {
            Err(unsupported())
        } else {
            Ok(())
        };
    }
    let prefix = format!("contract-attested-v1:{} ", t.public_model);
    let reference = t.source.strip_prefix(&prefix).ok_or_else(unsupported)?;
    if reference.trim().is_empty() {
        return Err(unsupported());
    }
    let c = &t.contract;
    use crate::models::platform_model_tariff::{CacheTtl, UsageFamily};
    let version = match c.family {
        UsageFamily::Gemini => "gemini.generate.v2",
        UsageFamily::Claude => "anthropic.messages.v2",
        UsageFamily::Openai => "openai.responses.v2",
        UsageFamily::OpenMaas => return Err(unsupported()),
    };
    if c.version != version
        || !c.tokens
        || !c.cache_read
        || c.images
        || !c.output_modalities.is_empty()
        || !(c.input_modalities.is_empty()
            || c.family == UsageFamily::Gemini && c.input_modalities == [UsageModality::Image])
        || c.family != UsageFamily::Gemini && c.bill_tool_input
        || c.family == UsageFamily::Claude && c.split_reasoning
        || c.cache_write_ttls.iter().any(|ttl| {
            !matches!(
                (c.family, ttl),
                (
                    UsageFamily::Claude,
                    CacheTtl::FiveMinutes | CacheTtl::OneHour
                ) | (
                    UsageFamily::Openai,
                    CacheTtl::ThirtyMinutes | CacheTtl::TwelveHours
                )
            )
        })
    {
        return Err(unsupported());
    }
    Ok(())
}

fn certify_family(
    native: Native,
    route: &RouteTarget,
    tariff: &ModelTariff,
    has_images: bool,
) -> AppResult<()> {
    validate_family_tariff(tariff)?;
    let transport_matches = matches!(
        (native, route),
        (
            Native::Gemini,
            RouteTarget::VertexPublisher {
                publisher: VertexPublisher::Google,
                ..
            }
        ) | (
            Native::Claude,
            RouteTarget::VertexPublisher {
                publisher: VertexPublisher::Anthropic,
                ..
            }
        ) | (
            Native::Claude,
            RouteTarget::DirectProvider {
                provider: DirectProvider::Anthropic,
                ..
            }
        ) | (
            Native::Responses,
            RouteTarget::DirectProvider {
                provider: DirectProvider::Openai,
                ..
            }
        )
    );
    if !transport_matches
        || native.family() != tariff.contract.family
        || has_images && tariff.bounds.max_images == 0
    {
        return Err(unsupported());
    }
    Ok(())
}
