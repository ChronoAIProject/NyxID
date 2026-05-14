use anyhow::{Result, bail};
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde::{Deserialize, Serialize};

use crate::api::ApiClient;
use crate::cli::{AuthArgs, DeviceCommands, OutputFormat};
use crate::org_resolver::resolve_org_id;

pub struct ApproveDeviceArgs {
    pub user_code: String,
    pub org: Option<String>,
    pub label: Option<String>,
    pub auth: AuthArgs,
}

#[derive(Debug, Serialize)]
struct ApproveDeviceRequest {
    user_code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    org_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ApproveDeviceResponse {
    device_label: String,
    hw_id: String,
    api_key_id: String,
    node_id: String,
    owner_user_id: String,
    org_id: Option<String>,
}

pub async fn run(command: DeviceCommands) -> Result<()> {
    match command {
        DeviceCommands::Approve {
            user_code,
            org,
            label,
            auth,
        } => {
            approve_cmd(ApproveDeviceArgs {
                user_code,
                org,
                label,
                auth,
            })
            .await
        }
    }
}

pub async fn approve_cmd(args: ApproveDeviceArgs) -> Result<()> {
    let normalized_user_code = normalize_user_code(&args.user_code)?;
    let mut api = ApiClient::from_auth(&args.auth)?;
    let org_id = match args.org {
        Some(raw) => Some(resolve_org_id(&mut api, &raw).await?),
        None => None,
    };

    let request = ApproveDeviceRequest {
        user_code: normalized_user_code,
        org_id,
        label: normalize_label(args.label)?,
    };
    let response: ApproveDeviceResponse = api.post("/devices/code/approve", &request).await?;

    match args.auth.output {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        OutputFormat::Table => print_approval_table(&response),
    }

    Ok(())
}

fn print_approval_table(response: &ApproveDeviceResponse) {
    let api_key_id = short_id(&response.api_key_id);
    let node_id = short_id(&response.node_id);
    let mut table = Table::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    table.set_header(["Device", "HW ID", "API Key", "Node", "Org"]);
    table.add_row([
        response.device_label.as_str(),
        response.hw_id.as_str(),
        api_key_id.as_str(),
        node_id.as_str(),
        response.org_id.as_deref().unwrap_or("personal"),
    ]);
    eprintln!("{table}");
    eprintln!("Device will pick up credentials on its next poll.");
}

fn normalize_label(value: Option<String>) -> Result<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.len() > 200 {
        bail!("Device label must be at most 200 characters");
    }
    Ok(Some(trimmed.to_string()))
}

pub(crate) fn normalize_user_code(value: &str) -> Result<String> {
    let compact = value
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '-')
        .collect::<String>()
        .to_ascii_uppercase();

    if compact.len() != 12 || !compact.bytes().all(is_user_code_byte) {
        bail!("Invalid user code. Expected 12 characters from ABCDEFGHJKLMNPQRSTUVWXYZ23456789");
    }

    Ok(format!(
        "{}-{}-{}",
        &compact[0..4],
        &compact[4..8],
        &compact[8..12]
    ))
}

fn is_user_code_byte(byte: u8) -> bool {
    matches!(
        byte,
        b'A' | b'B'
            | b'C'
            | b'D'
            | b'E'
            | b'F'
            | b'G'
            | b'H'
            | b'J'
            | b'K'
            | b'L'
            | b'M'
            | b'N'
            | b'P'
            | b'Q'
            | b'R'
            | b'S'
            | b'T'
            | b'U'
            | b'V'
            | b'W'
            | b'X'
            | b'Y'
            | b'Z'
            | b'2'
            | b'3'
            | b'4'
            | b'5'
            | b'6'
            | b'7'
            | b'8'
            | b'9'
    )
}

fn short_id(value: &str) -> String {
    if value.len() <= 12 {
        value.to_string()
    } else {
        let prefix = value.chars().take(8).collect::<String>();
        format!("{prefix}...")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_user_code_accepts_compact_dashed_and_spaced_forms() {
        assert_eq!(
            normalize_user_code("abcd efgh jklm").unwrap(),
            "ABCD-EFGH-JKLM"
        );
        assert_eq!(
            normalize_user_code("abcd-efgh-jklm").unwrap(),
            "ABCD-EFGH-JKLM"
        );
        assert_eq!(
            normalize_user_code("abcdefghjklm").unwrap(),
            "ABCD-EFGH-JKLM"
        );
    }

    #[test]
    fn normalize_user_code_rejects_ambiguous_letters_and_bad_lengths() {
        assert!(normalize_user_code("ABCD-EFGH-IJKL").is_err());
        assert!(normalize_user_code("ABCD-EFGH-OJKL").is_err());
        assert!(normalize_user_code("ABCD-EFGH-JKL").is_err());
    }

    #[test]
    fn normalize_label_trims_and_caps_length() {
        assert_eq!(
            normalize_label(Some(" Hall ".to_string())).unwrap(),
            Some("Hall".to_string())
        );
        assert_eq!(normalize_label(Some("  ".to_string())).unwrap(), None);
        assert!(normalize_label(Some("x".repeat(201))).is_err());
    }

    #[test]
    fn short_id_truncates_long_ids() {
        assert_eq!(short_id("12345678-1234"), "12345678...");
        assert_eq!(short_id("short"), "short");
    }
}
