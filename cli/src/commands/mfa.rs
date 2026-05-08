use anyhow::Result;
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::{MfaCommands, OutputFormat};

pub async fn run(command: MfaCommands) -> Result<()> {
    match command {
        MfaCommands::Setup {
            terminal,
            no_wait,
            auth,
        } => {
            // Browser-flow gate: open the local wizard when a browser
            // is available, fall through to the remote-pairing
            // transport otherwise. The wizard runs BOTH halves of
            // enrollment (setup + confirm) in the browser, so neither
            // the TOTP secret nor the recovery codes ever land in the
            // terminal. `--terminal` and `NYXID_NO_WIZARD=1` opt out
            // and use the legacy in-terminal output below.
            //
            // `--no-wait` always picks the pairing transport (matches
            // the `api-key create/rotate` UX).
            let interactive_output = matches!(auth.output, OutputFormat::Table);
            let wizard_eligible = !terminal
                && (no_wait || (interactive_output && crate::wizard::is_browser_flow_eligible()));

            if wizard_eligible {
                let prefill = crate::wizard::MfaSetupPrefill {};
                return crate::wizard::run_mfa_setup_wizard(&auth, prefill, no_wait).await;
            }

            // Scripted path — preserved byte-identical to the
            // pre-wizard behavior so existing CI / scripts keep
            // working. Note this prints the TOTP secret + URL to
            // the terminal; that's exactly the leak the wizard
            // closes for the default interactive path.
            let mut api = ApiClient::from_auth(&auth)?;
            let result: Value = api.post("/auth/mfa/setup", &serde_json::json!({})).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    let secret = result["secret"].as_str().unwrap_or("-");
                    let qr_url = result["qr_code_url"]
                        .as_str()
                        .or(result["otpauth_url"].as_str())
                        .unwrap_or("-");

                    eprintln!("MFA Setup");
                    eprintln!();
                    eprintln!("Secret:  {secret}");
                    eprintln!("QR URL:  {qr_url}");
                    eprintln!();
                    eprintln!("1. Add this secret to your authenticator app (or scan the QR URL)");
                    eprintln!("2. Verify with: nyxid mfa verify --code <TOTP_CODE>");
                }
            }
            Ok(())
        }

        MfaCommands::Verify { code, auth } => {
            let mut api = ApiClient::from_auth(&auth)?;
            let body = serde_json::json!({ "code": code });
            // Backend route is `/auth/mfa/confirm` — MFA endpoints are
            // nested under `/auth` in `backend/src/routes.rs:63`. The
            // previous CLI used the non-existent `/mfa/verify-setup`
            // path, so this scripted command was broken pre-#506.
            let result: Value = api.post("/auth/mfa/confirm", &body).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    eprintln!("MFA enabled successfully.");
                    if let Some(codes) = result["recovery_codes"].as_array()
                        && !codes.is_empty()
                    {
                        eprintln!();
                        eprintln!("Recovery codes (save these securely):");
                        for code in codes {
                            if let Some(c) = code.as_str() {
                                eprintln!("  {c}");
                            }
                        }
                    }
                }
            }
            Ok(())
        }

        MfaCommands::Status { auth } => {
            let mut api = ApiClient::from_auth(&auth)?;
            let user: Value = api.get("/users/me").await?;

            match auth.output {
                OutputFormat::Json => {
                    let status = serde_json::json!({
                        "mfa_enabled": user["mfa_enabled"],
                    });
                    println!("{}", serde_json::to_string_pretty(&status)?);
                }
                OutputFormat::Table => {
                    let enabled = user["mfa_enabled"].as_bool().unwrap_or(false);
                    let status = if enabled { "enabled" } else { "disabled" };
                    eprintln!("MFA: {status}");
                    if !enabled {
                        eprintln!();
                        eprintln!("Enable with: nyxid mfa setup");
                    }
                }
            }
            Ok(())
        }
    }
}
