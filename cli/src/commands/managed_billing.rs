use anyhow::{Context, Result, bail};
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};

use super::billing_support::{credits, price_pico, rfc3339, timestamp};
use crate::cli::ManagedBillingArgs;

pub(super) fn body(args: &ManagedBillingArgs, now: DateTime<Utc>) -> Result<(Value, String)> {
    if args.model.is_empty()
        || args.model.len() > 200
        || !args
            .model
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/._:-".contains(&b))
    {
        bail!("model must be an exact model ID of 1..=200 ASCII letters, digits or /._:-");
    }
    for (name, value, low, high) in [
        ("max-input-tokens", args.max_input_tokens, 1, 2_000_000),
        ("max-output-tokens", args.max_output_tokens, 1, 32_768),
        ("max-request-bytes", args.max_request_bytes, 256, 1_048_576),
        (
            "max-response-bytes",
            args.max_response_bytes,
            1024,
            8_388_608,
        ),
    ] {
        if !(low..=high).contains(&value) {
            bail!("{name} must be in {low}..={high}");
        }
    }
    if args.max_output_tokens >= args.max_input_tokens {
        bail!("Output cap must be below the full context ceiling");
    }
    if args.tariff_version.trim().is_empty() || args.tariff_version.len() > 200 {
        bail!("tariff-version must contain 1..=200 bytes");
    }
    let source = url::Url::parse(&args.tariff_source).ok();
    if source.is_none_or(|url| {
        url.scheme() != "https"
            || !matches!(
                url.host_str(),
                Some("telnyx.com" | "api.telnyx.com" | "developers.telnyx.com")
            )
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
    }) || args.tariff_source.len() > 2048
    {
        bail!(
            "tariff-source must be a Telnyx HTTPS evidence URL without credentials, query or fragment"
        );
    }
    let verified = timestamp(&args.tariff_verified_at)?;
    if verified > now || verified < now - Duration::days(30) {
        bail!("Tariff verification must be in the past and at most 30 days old");
    }
    let mut rates = vec![];
    for (name, raw, positive) in [
        ("input-price", &args.input_price, true),
        ("output-price", &args.output_price, true),
        ("cache-read-price", &args.cache_read_price, false),
    ] {
        if raw != raw.trim() {
            bail!("{name} must be an exact decimal without whitespace");
        }
        let pico = price_pico(raw).with_context(|| format!("Invalid {name}"))?;
        if positive && pico == 0 {
            bail!("{name} must be positive");
        }
        rates.push(pico);
    }
    let ceiling = rates[0]
        .checked_mul(i128::from(args.max_input_tokens))
        .and_then(|v| {
            rates[1]
                .checked_mul(i128::from(args.max_output_tokens))
                .and_then(|n| v.checked_add(n))
        })
        .and_then(|v| {
            rates[2]
                .checked_mul(i128::from(args.max_input_tokens))
                .and_then(|n| v.checked_add(n))
        })
        .context("Reservation ceiling is out of range")?;
    Ok((
        json!({
            "billing": {
                "platform_billable": true,
                "platform_charge_nyxid_credentials_only": true,
                "managed_execution": {
                    "model": args.model,
                    "max_input_tokens": args.max_input_tokens,
                    "max_output_tokens": args.max_output_tokens,
                    "max_request_bytes": args.max_request_bytes,
                    "max_response_bytes": args.max_response_bytes,
                    "tariff_version": args.tariff_version,
                    "tariff_source": args.tariff_source,
                    "tariff_verified_at": {
                        "$date": rfc3339(verified)
                    }
                },
                "platform_key_pricing": {
                    "metric": "input_tokens",
                    "credits_per_unit": args.input_price,
                    "operations": [],
                    "components": [
                        {
                            "metric": "output_tokens",
                            "credits_per_unit": args.output_price
                        },
                        {
                            "metric": "cache_read_tokens",
                            "credits_per_unit": args.cache_read_price
                        }
                    ]
                },
                "resale_billable": false,
                "byok_pricing": null,
                "platform_pricing": null
            }
        }),
        credits(ceiling),
    ))
}

pub(super) fn validate_probe(response: &Value, args: &ManagedBillingArgs) -> Result<()> {
    if response["model"] != args.model {
        bail!("Probe must return the exact configured model ID");
    }
    if !response["id"].as_str().is_some_and(|id| {
        !id.is_empty()
            && id.len() <= 256
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    }) {
        bail!("Probe must contain a compatible provider request ID");
    }
    let usage = &response["usage"];
    let number = |v: &Value| {
        v.as_u64().filter(|n| *n <= i64::MAX as u64).context(concat!(
            "Probe needs nonnegative integer prompt, completion, total and cached token counts; ",
            "missing cached_tokens is not zero"
        ))
    };
    let prompt = number(&usage["prompt_tokens"])?;
    let completion = number(&usage["completion_tokens"])?;
    let total = number(&usage["total_tokens"])?;
    let cached = number(&usage["prompt_tokens_details"]["cached_tokens"])?;
    if cached > prompt
        || prompt.checked_add(completion) != Some(total)
        || total > args.max_input_tokens
        || completion > args.max_output_tokens
    {
        bail!("Probe token counts violate the managed contract");
    }
    let choices = response["choices"]
        .as_array()
        .context("Probe must have exactly one text assistant choice")?;
    if choices.len() != 1 {
        bail!("Probe must have exactly one text assistant choice");
    }
    let message = &choices[0]["message"];
    if message["role"] != "assistant"
        || !message
            .get("content")
            .is_some_and(|v| v.is_string() || v.is_null())
        || message.get("tool_calls").is_some()
    {
        bail!("Probe must be a text assistant response without tool_calls");
    }
    for (group, names) in [
        (
            usage,
            &["cache_write_tokens", "cache_creation_input_tokens"][..],
        ),
        (
            &usage["prompt_tokens_details"],
            &[
                "audio_tokens",
                "image_tokens",
                "cache_write_tokens",
                "cache_creation_tokens",
            ][..],
        ),
        (
            &usage["completion_tokens_details"],
            &["audio_tokens", "image_tokens"][..],
        ),
        (&usage["input_tokens_details"], &["cache_write_tokens"][..]),
    ] {
        for name in names {
            if group.get(*name).is_some_and(|v| v.as_u64() != Some(0)) {
                bail!("Probe reports unsupported token modalities");
            }
        }
    }
    Ok(())
}
