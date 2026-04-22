use anyhow::Result;

use crate::cli::{ConfigCommands, ConfigKey, ConfigToggleValue};

pub async fn run(command: ConfigCommands) -> Result<()> {
    match command {
        ConfigCommands::Set {
            key,
            value,
            profile,
        } => {
            let enabled = matches!(value, ConfigToggleValue::On);

            match key {
                ConfigKey::UpdateCheck => {
                    crate::settings::set_update_check(profile.as_deref(), enabled)?;
                    eprintln!(
                        "Set update-check={}{}.",
                        if enabled { "on" } else { "off" },
                        profile
                            .as_deref()
                            .map(|profile| format!(" for profile {profile}"))
                            .unwrap_or_default()
                    );
                }
            }
        }
    }

    Ok(())
}
