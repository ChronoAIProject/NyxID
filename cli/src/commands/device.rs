use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};

use crate::api::ApiClient;
use crate::cli::{AuthArgs, DeviceCommands, OutputFormat};
use crate::org_resolver::resolve_org_id;

pub struct ApproveDeviceArgs {
    pub user_code: String,
    pub org: Option<String>,
    pub label: Option<String>,
    pub service: Vec<String>,
    pub auth: AuthArgs,
}

pub struct FactoryKeyArgs {
    pub count: usize,
    pub out: Option<PathBuf>,
    pub ndjson: bool,
}

#[derive(Debug, Serialize)]
struct ApproveDeviceRequest {
    user_code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    org_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_services: Option<Vec<String>>,
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

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct FactoryKey {
    pubkey_hex: String,
    privkey_hex: String,
}

pub async fn run(command: DeviceCommands) -> Result<()> {
    match command {
        DeviceCommands::Approve {
            user_code,
            org,
            label,
            service,
            auth,
        } => {
            approve_cmd(ApproveDeviceArgs {
                user_code,
                org,
                label,
                service,
                auth,
            })
            .await
        }
        DeviceCommands::FactoryKey { count, out, ndjson } => {
            factory_key_cmd(FactoryKeyArgs { count, out, ndjson })
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
        default_services: normalize_default_services(args.service)?,
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

pub fn factory_key_cmd(args: FactoryKeyArgs) -> Result<()> {
    if args.count == 0 {
        bail!("--count must be at least 1");
    }

    let keys = generate_factory_keys(args.count);
    let output = render_factory_keys(&keys, args.ndjson)?;

    match args.out {
        Some(path) => write_factory_key_output(&path, output.as_bytes()),
        None => {
            print!("{output}");
            Ok(())
        }
    }
}

fn generate_factory_keys(count: usize) -> Vec<FactoryKey> {
    let mut rng = OsRng;
    (0..count)
        .map(|_| {
            let signing_key = SigningKey::generate(&mut rng);
            let verifying_key = signing_key.verifying_key();
            FactoryKey {
                pubkey_hex: hex::encode(verifying_key.to_bytes()),
                privkey_hex: hex::encode(signing_key.to_bytes()),
            }
        })
        .collect()
}

fn render_factory_keys(keys: &[FactoryKey], ndjson: bool) -> Result<String> {
    if ndjson {
        let mut out = String::new();
        for key in keys {
            out.push_str(&serde_json::to_string(key)?);
            out.push('\n');
        }
        return Ok(out);
    }

    let mut out = serde_json::to_string_pretty(keys)?;
    out.push('\n');
    Ok(out)
}

fn write_factory_key_output(path: &Path, contents: &[u8]) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("Failed to create {}", path.display()))?;
        file.write_all(contents)
            .with_context(|| format!("Failed to write {}", path.display()))?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("Failed to set permissions on {}", path.display()))?;
    }

    #[cfg(not(unix))]
    {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)
            .with_context(|| format!("Failed to create {}", path.display()))?;
        file.write_all(contents)
            .with_context(|| format!("Failed to write {}", path.display()))?;
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

fn normalize_default_services(values: Vec<String>) -> Result<Option<Vec<String>>> {
    let mut normalized = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            bail!("--service values must not be empty");
        }
        normalized.push(trimmed.to_string());
    }

    if normalized.is_empty() {
        Ok(None)
    } else {
        Ok(Some(normalized))
    }
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
    fn normalize_default_services_omits_empty_list_and_trims_values() {
        assert_eq!(normalize_default_services(Vec::new()).unwrap(), None);
        assert_eq!(
            normalize_default_services(vec![
                " llm-openai ".to_string(),
                "550e8400-e29b-41d4-a716-446655440000".to_string(),
            ])
            .unwrap(),
            Some(vec![
                "llm-openai".to_string(),
                "550e8400-e29b-41d4-a716-446655440000".to_string(),
            ])
        );
        assert!(normalize_default_services(vec!["  ".to_string()]).is_err());
    }

    #[test]
    fn short_id_truncates_long_ids() {
        assert_eq!(short_id("12345678-1234"), "12345678...");
        assert_eq!(short_id("short"), "short");
    }

    #[test]
    fn generate_factory_keys_returns_32_byte_hex_fields() {
        let keys = generate_factory_keys(2);

        assert_eq!(keys.len(), 2);
        for key in keys {
            assert_eq!(key.pubkey_hex.len(), 64);
            assert_eq!(key.privkey_hex.len(), 64);
            assert_eq!(hex::decode(&key.pubkey_hex).unwrap().len(), 32);
            assert_eq!(hex::decode(&key.privkey_hex).unwrap().len(), 32);
        }
    }

    #[test]
    fn render_factory_keys_defaults_to_json_array() {
        let keys = vec![FactoryKey {
            pubkey_hex: "a".repeat(64),
            privkey_hex: "b".repeat(64),
        }];

        let rendered = render_factory_keys(&keys, false).unwrap();
        let parsed: Vec<FactoryKey> = serde_json::from_str(&rendered).unwrap();

        assert_eq!(parsed, keys);
        assert!(rendered.starts_with("["));
        assert!(rendered.ends_with('\n'));
    }

    #[test]
    fn render_factory_keys_supports_ndjson() {
        let keys = vec![
            FactoryKey {
                pubkey_hex: "a".repeat(64),
                privkey_hex: "b".repeat(64),
            },
            FactoryKey {
                pubkey_hex: "c".repeat(64),
                privkey_hex: "d".repeat(64),
            },
        ];

        let rendered = render_factory_keys(&keys, true).unwrap();
        let lines = rendered.lines().collect::<Vec<_>>();

        assert_eq!(lines.len(), 2);
        assert_eq!(
            serde_json::from_str::<FactoryKey>(lines[0]).unwrap(),
            keys[0]
        );
        assert_eq!(
            serde_json::from_str::<FactoryKey>(lines[1]).unwrap(),
            keys[1]
        );
    }

    #[test]
    fn factory_key_cmd_rejects_zero_count() {
        let error = factory_key_cmd(FactoryKeyArgs {
            count: 0,
            out: None,
            ndjson: false,
        })
        .unwrap_err();

        assert!(error.to_string().contains("at least 1"));
    }

    #[cfg(unix)]
    #[test]
    fn factory_key_cmd_writes_output_file_with_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("factory-keys.json");

        factory_key_cmd(FactoryKeyArgs {
            count: 1,
            out: Some(path.clone()),
            ndjson: false,
        })
        .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: Vec<FactoryKey> = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
