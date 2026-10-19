//! Shared human-session guard, exact decimals and billing output helpers.
use anyhow::{Context, Result, bail};
use chrono::{DateTime, SecondsFormat, Utc};
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::{Value, json};

use crate::{
    api::ApiClient,
    cli::{AuthArgs, OutputFormat},
};

pub(crate) fn require_human(auth: &AuthArgs) -> Result<()> {
    let is_api_key = |token: &str| {
        let token = token.trim();
        token.starts_with("nyx_") || token.starts_with("nyxid_")
    };
    let agent_credential = match crate::auth::resolve_access_token_override(auth) {
        Some(resolved) => is_api_key(&resolved.token),
        None => {
            crate::auth::agent_key::is_agent_key_profile(auth.profile.as_deref())
                || crate::auth::read_saved_token_for(auth.profile.as_deref())
                    .is_some_and(|token| is_api_key(&token))
        }
    };
    if agent_credential {
        bail!(
            "This command requires a human account session. Run `nyxid login` with a human account, \
             using a separate --profile from your Agent Key."
        );
    }
    Ok(())
}

pub(crate) async fn human_api(auth: &AuthArgs) -> Result<ApiClient> {
    require_human(auth)?;
    Ok(ApiClient::from_auth_checked(auth)
        .await?
        .without_write_refresh())
}

pub(crate) async fn service(api: &mut ApiClient, reference: &str) -> Result<Value> {
    let row = super::service::catalog_admin::fetch_catalog_service(api, reference).await?;
    uuid(row["id"].as_str().context("Catalog service has no UUID")?)?;
    Ok(row)
}

pub(crate) fn uuid(raw: &str) -> Result<String> {
    uuid::Uuid::parse_str(raw)
        .map(|id| id.to_string())
        .context("Expected a UUID")
}

pub(crate) fn timestamp(raw: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .map(|date| date.with_timezone(&Utc))
        .context("Expected an RFC3339 timestamp with a timezone")
}

pub(crate) fn bson_date(value: &Value) -> Option<DateTime<Utc>> {
    if let Some(raw) = value.as_str() {
        return timestamp(raw).ok();
    }
    let date = value.get("$date")?;
    if let Some(raw) = date.as_str() {
        return timestamp(raw).ok();
    }
    let millis = date
        .get("$numberLong")
        .and_then(Value::as_str)
        .and_then(|raw| raw.parse().ok())
        .or_else(|| date.as_i64())?;
    DateTime::from_timestamp_millis(millis)
}

pub(crate) fn rfc3339(date: DateTime<Utc>) -> String {
    date.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

pub(crate) fn price_pico(raw: &str) -> Result<i128> {
    let price = super::billing_units::price(raw).map_err(anyhow::Error::msg)?;
    decimal_pico(&price)
}

pub(crate) fn decimal_pico(raw: &str) -> Result<i128> {
    let (whole, fraction) = raw.split_once('.').unwrap_or((raw, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > 12
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || (raw.contains('.') && fraction.is_empty())
    {
        bail!("Expected an exact nonnegative decimal with at most 12 fractional digits");
    }
    whole
        .parse::<i128>()
        .ok()
        .and_then(|n| n.checked_mul(1_000_000_000_000))
        .and_then(|n| {
            fraction
                .parse::<i128>()
                .unwrap_or(0)
                .checked_mul(10_i128.pow((12 - fraction.len()) as u32))
                .and_then(|f| n.checked_add(f))
        })
        .context("Exact decimal is out of range")
}

pub(crate) fn credits(pico: i128) -> String {
    let magnitude = pico.unsigned_abs();
    let sign = if pico < 0 { "-" } else { "" };
    let whole = magnitude / 1_000_000_000_000;
    let fraction = magnitude % 1_000_000_000_000;
    if fraction == 0 {
        format!("{sign}{whole}")
    } else {
        format!("{sign}{whole}.{fraction:012}")
            .trim_end_matches('0')
            .to_string()
    }
}

pub(crate) fn text(value: &Value) -> String {
    match value {
        Value::Null => "-".into(),
        Value::String(s) => s.clone(),
        _ => value.to_string(),
    }
}

pub(crate) fn table(headers: &[&str], rows: impl IntoIterator<Item = Vec<String>>) -> String {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    table.set_header(headers);
    for row in rows {
        table.add_row(row);
    }
    table.to_string()
}

pub(crate) fn print(
    value: &Value,
    output: OutputFormat,
    human: impl FnOnce() -> String,
) -> Result<()> {
    if matches!(output, OutputFormat::Json) {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        eprintln!("{}", human());
    }
    Ok(())
}

pub(crate) fn fields(value: &Value) -> String {
    fn flatten(value: &Value, path: &str, rows: &mut Vec<Vec<String>>) {
        match value {
            Value::Object(object) if !object.is_empty() => {
                if let Some(date) = bson_date(value) {
                    rows.push(vec![path.into(), rfc3339(date)]);
                    return;
                }
                for (key, value) in object {
                    flatten(
                        value,
                        &if path.is_empty() {
                            key.clone()
                        } else {
                            format!("{path}.{key}")
                        },
                        rows,
                    );
                }
            }
            Value::Array(array) if !array.is_empty() => {
                for (i, value) in array.iter().enumerate() {
                    flatten(value, &format!("{path}[{i}]"), rows);
                }
            }
            _ => rows.push(vec![path.into(), text(value)]),
        }
    }
    let mut rows = vec![];
    flatten(value, "", &mut rows);
    table(&["Field", "Value"], rows)
}

pub(crate) fn pending_id(message: &str) -> Option<String> {
    let rest = message.split_once("Managed execution ")?.1;
    let (id, tail) = rest.split_once(" is pending usage verification")?;
    if tail.starts_with(';') || tail.is_empty() {
        uuid(id).ok()
    } else {
        None
    }
}

pub(crate) fn pending(id: String, output: OutputFormat) -> Result<()> {
    let report = json!({"error":"managed_execution_pending","execution_id":id});
    print(&report, output, || {
        format!(
            "Execution: {id}\nManaged execution is pending usage verification. Inspect its status; do not retry automatically."
        )
    })?;
    Err(super::admin_tools::PendingReadiness { exit_code: 2 }.into())
}

pub(crate) fn exit_if_not_ready(ready: bool) -> Result<()> {
    if ready {
        Ok(())
    } else {
        Err(super::admin_tools::PendingReadiness { exit_code: 2 }.into())
    }
}
