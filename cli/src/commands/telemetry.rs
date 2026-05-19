//! `nyxid telemetry {enable,disable,status}` — the canonical editor for
//! the persisted telemetry consent flag at `~/.nyxid/config.toml`.
//!
//! `enable`  → `{enabled=true,  asked=true}` in config
//! `disable` → `{enabled=false, asked=true}` in config + deletes any
//!             cached anon UUID so a re-enable produces a fresh trail
//! `status`  → prints the resolved state and where it came from

use anyhow::Result;

use crate::cli::TelemetryCommands;
use crate::telemetry;

pub async fn run(command: TelemetryCommands, profile: Option<&str>) -> Result<()> {
    match command {
        TelemetryCommands::Enable => {
            telemetry::consent::persist_choice(profile, true)?;
            eprintln!("Telemetry enabled. Run `nyxid telemetry status` to confirm.");
            Ok(())
        }
        TelemetryCommands::Disable => {
            telemetry::consent::persist_choice(profile, false)?;
            // Wipe any cached anon UUID so re-enable starts fresh.
            if let Some(path) = telemetry::anon_id_path(profile) {
                let _ = std::fs::remove_file(path);
            }
            eprintln!("Telemetry disabled. No events will be sent.");
            Ok(())
        }
        TelemetryCommands::Status => {
            let state = telemetry::consent::resolve_consent(profile);
            let source = match state.source {
                telemetry::consent::ConsentSource::DoNotTrack => {
                    "env: DO_NOT_TRACK is set (forced disable, not persisted)"
                }
                telemetry::consent::ConsentSource::EnvVarOff => {
                    "env: NYXID_TELEMETRY=off (forced disable)"
                }
                telemetry::consent::ConsentSource::EnvVarOn => {
                    "env: NYXID_TELEMETRY=on (forced enable)"
                }
                telemetry::consent::ConsentSource::ConfigEnabled => {
                    "config: ~/.nyxid/config.toml (enabled=true)"
                }
                telemetry::consent::ConsentSource::ConfigDeclined => {
                    "config: ~/.nyxid/config.toml (enabled=false)"
                }
                telemetry::consent::ConsentSource::FirstRunPending => {
                    "default: no choice persisted yet"
                }
            };
            let enabled_str = if state.enabled { "ON" } else { "OFF" };
            println!("Telemetry: {enabled_str}");
            println!("Source:    {source}");
            println!("Persisted: {}", state.persisted);
            if state.needs_prompt {
                println!(
                    "(A prompt will appear on the next interactive run to persist this choice.)"
                );
            }
            Ok(())
        }
    }
}
