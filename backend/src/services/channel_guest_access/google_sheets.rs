//! Exact-resource authorization for the hosted Sheets values API. The connection,
//! canonical path and bytes come from the existing proxy/MCP resolver.
use crate::services::{content_type::is_json_content_type, proxy_authorization::CanonicalPath};
use reqwest::Method;
use serde::Deserialize;

pub(crate) const SOURCE_ID: &str = "google_sheets";
pub(crate) const COLLECTION_ID: &str = "values";
pub(crate) const ACTIONS: [&str; 2] = ["read", "create"];
const MAX_APPEND_BYTES: usize = 16 * 1024;
const MAX_COLUMNS: u32 = 32;
const MAX_READ_CELLS: u32 = 2000;

pub(crate) struct SheetsOperation {
    pub(crate) action: &'static str,
    pub(crate) resource_id: String,
}

pub(crate) fn supports_destination(base_url: &str) -> bool {
    matches!(
        base_url,
        "https://sheets.googleapis.com" | "https://sheets.googleapis.com/"
    )
}

pub(crate) fn supports_catalog(slug: &str) -> bool {
    matches!(slug, "api-google-sheets" | "api-google-workspace")
}

pub(crate) fn validate_resource_id(resource_id: &str, action: &str) -> bool {
    let parts = resource_id.split('/').collect::<Vec<_>>();
    let [connection_id, workbook_id, selector] = parts.as_slice() else {
        return false;
    };
    valid_segment(connection_id)
        && valid_segment(workbook_id)
        && match action {
            "read" => read_range(selector).is_some(),
            "create" => valid_tab_title(selector),
            _ => false,
        }
}

/// Check after standard input normalization. Callers enforce the ordinary
/// service, destination, guest-operation and live approval ceilings separately.
pub(crate) fn classify(
    catalog_slug: &str,
    connection_id: &str,
    method: &Method,
    path: &CanonicalPath,
    query: Option<&str>,
    body: &[u8],
    content_type: Option<&str>,
) -> Option<SheetsOperation> {
    if !supports_catalog(catalog_slug) || !valid_segment(connection_id) {
        return None;
    }
    let canonical = path.as_policy_path();
    let mut segments = canonical.strip_prefix("/v4/spreadsheets/")?.split('/');
    let workbook = segments.next().filter(|segment| valid_segment(segment))?;
    if segments.next()? != "values" {
        return None;
    }
    let range_segment = segments.next()?;
    if segments.next().is_some() {
        return None;
    }

    let parameters = nyxid_permissions::parse_query(query.unwrap_or_default()).ok()?;
    let (action, selector) = match *method {
        Method::GET
            if body.is_empty()
                && parameters.iter().all(|(key, value)| {
                    matches!(
                        (key.as_str(), value.as_str()),
                        ("majorDimension", "ROWS" | "COLUMNS")
                            | ("valueRenderOption", "FORMATTED_VALUE" | "UNFORMATTED_VALUE")
                            | ("dateTimeRenderOption", "SERIAL_NUMBER" | "FORMATTED_STRING")
                    )
                }) =>
        {
            read_range(range_segment)?;
            ("read", range_segment)
        }
        Method::POST => {
            let range = range_segment.strip_suffix(":append")?;
            let (tab, columns) = sheet_range(range)?;
            let (first, last) = columns.split_once(':')?;
            let width = column(last)?.checked_sub(column(first)?)? + 1;
            if parameters.get("insertDataOption").map(String::as_str) != Some("INSERT_ROWS")
                || parameters.get("valueInputOption").map(String::as_str) != Some("RAW")
                || !parameters.iter().all(|(key, value)| {
                    matches!(
                        (key.as_str(), value.as_str()),
                        ("insertDataOption", "INSERT_ROWS")
                            | ("valueInputOption", "RAW")
                            | ("includeValuesInResponse", "false")
                    )
                })
                || !content_type.is_some_and(is_json_content_type)
                || width > MAX_COLUMNS
                || !valid_append_body(body, width)
            {
                return None;
            }
            // The supplied column range locates a table. Google chooses
            // placement within the tab, so the policy must grant the tab.
            ("create", tab)
        }
        _ => return None,
    };
    Some(SheetsOperation {
        action,
        resource_id: format!("{connection_id}/{workbook}/{selector}"),
    })
}

fn valid_segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

fn valid_tab_title(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.chars().count() <= 100
        && value
            .chars()
            .all(|ch| ch.is_alphanumeric() || matches!(ch, ' ' | '_' | '-'))
}

fn sheet_range(range: &str) -> Option<(&str, &str)> {
    let (sheet, cells) = range.split_once('!')?;
    if let Some(title) = sheet
        .strip_prefix('\'')
        .and_then(|name| name.strip_suffix('\''))
    {
        return valid_tab_title(title).then_some((title, cells));
    }
    (valid_tab_title(sheet) && sheet.chars().all(|ch| ch.is_alphanumeric() || ch == '_'))
        .then_some((sheet, cells))
}

fn read_range(range: &str) -> Option<()> {
    let (_, cells) = sheet_range(range)?;
    let (first, last) = cells.split_once(':').unwrap_or((cells, cells));
    let (left, top) = cell(first)?;
    let (right, bottom) = cell(last)?;
    let width = right.checked_sub(left)? + 1;
    let height = bottom.checked_sub(top)? + 1;
    (width <= MAX_COLUMNS && width.checked_mul(height)? <= MAX_READ_CELLS).then_some(())
}

fn column(value: &str) -> Option<u32> {
    if value.is_empty() || value.len() > 3 || !value.bytes().all(|b| b.is_ascii_uppercase()) {
        return None;
    }
    Some(
        value
            .bytes()
            .fold(0, |n, ch| n * 26 + u32::from(ch - b'A') + 1),
    )
}

fn cell(value: &str) -> Option<(u32, u32)> {
    let first_digit = value.find(|ch: char| ch.is_ascii_digit())?;
    let (letters, digits) = value.split_at(first_digit);
    if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((column(letters)?, digits.parse::<u32>().ok()?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppendRow {
    #[serde(rename = "majorDimension")]
    major_dimension: Option<String>,
    values: Vec<Vec<serde_json::Value>>,
}

fn valid_append_body(body: &[u8], width: u32) -> bool {
    if body.len() > MAX_APPEND_BYTES
        || body.iter().find(|byte| !byte.is_ascii_whitespace()) != Some(&b'{')
    {
        return false;
    }
    let Ok(append) = serde_json::from_slice::<AppendRow>(body) else {
        return false;
    };
    let [row] = append.values.as_slice() else {
        return false;
    };
    append
        .major_dimension
        .as_deref()
        .is_none_or(|dimension| dimension == "ROWS")
        && !row.is_empty()
        && row.len() <= width as usize
        && row
            .iter()
            .all(|cell| cell.is_string() || cell.is_number() || cell.is_boolean())
}

#[cfg(test)]
mod tests;
