use anyhow::Result;
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::{OutputFormat, ProviderCommands};
use crate::org_resolver::resolve_org_id;

pub async fn run(command: ProviderCommands) -> Result<()> {
    match command {
        ProviderCommands::Disconnect {
            provider_id,
            org,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let org = match org {
                Some(raw) => Some(resolve_org_id(&mut api, &raw).await?),
                None => None,
            };
            let path = disconnect_path(&provider_id, org.as_deref());
            let result: Value = api.delete(&path).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    let message = result["message"]
                        .as_str()
                        .unwrap_or("Provider disconnected and credentials removed");
                    let status = result["status"].as_str().unwrap_or("disconnected");
                    eprintln!("{message}");
                    eprintln!("Status: {status}");
                    if let Some(org_id) = org.as_deref() {
                        eprintln!("Org:    {org_id}");
                    }
                }
            }

            Ok(())
        }
    }
}

fn disconnect_path(provider_id: &str, target_org_id: Option<&str>) -> String {
    let mut path = format!("/providers/{provider_id}/disconnect");
    if let Some(org_id) = target_org_id {
        path.push_str("?target_org_id=");
        path.push_str(&urlencoding::encode(org_id));
    }
    path
}

#[cfg(test)]
mod tests {
    use super::disconnect_path;

    #[test]
    fn disconnect_path_omits_target_org_when_absent() {
        assert_eq!(
            disconnect_path("provider-1", None),
            "/providers/provider-1/disconnect"
        );
    }

    #[test]
    fn disconnect_path_appends_encoded_target_org() {
        assert_eq!(
            disconnect_path("provider-1", Some("org 1&2")),
            "/providers/provider-1/disconnect?target_org_id=org%201%262"
        );
    }
}
