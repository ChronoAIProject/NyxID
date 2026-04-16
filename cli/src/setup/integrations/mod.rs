pub mod ha_addon;

use serde_json::Value;

/// Trait for integration-specific setup wizard configuration.
///
/// Each integration returns a JSON config that the single `setup.html` page
/// renders dynamically. Adding a new integration means implementing this trait
/// — no HTML changes required.
pub trait IntegrationAdapter: Send + Sync {
    /// Machine-readable type identifier (e.g. "ha-addon").
    #[allow(dead_code)]
    fn integration_type(&self) -> &str;

    /// Human-readable name shown in the wizard header.
    fn display_name(&self) -> &str;

    /// JSON config describing wizard steps and fields.
    ///
    /// Step types:
    /// - `auto`    — executes an API call via the proxy automatically
    /// - `input`   — shows form fields, waits for user submission
    /// - `display` — shows results/credentials from a previous step
    /// - `confirm` — shows summary, user clicks to proceed
    fn config(&self) -> Value;
}

/// Generic interactive adapter — creates an API key with no target-specific logic.
pub struct InteractiveAdapter;

impl IntegrationAdapter for InteractiveAdapter {
    fn integration_type(&self) -> &str {
        "interactive"
    }

    fn display_name(&self) -> &str {
        "Interactive Setup"
    }

    fn config(&self) -> Value {
        serde_json::json!({
            "type": "interactive",
            "name": "Interactive Setup",
            "steps": [
                {
                    "id": "create_key",
                    "label": "Create NyxID API Key",
                    "type": "auto",
                    "action": {
                        "method": "POST",
                        "path": "/api-keys",
                        "body": {
                            "name": "Setup Wizard Key"
                        }
                    }
                },
                {
                    "id": "show_key",
                    "label": "Your Credentials",
                    "type": "display",
                    "source_step": "create_key",
                    "instructions": "Copy the API key above and store it securely. It will not be shown again."
                },
                {
                    "id": "done",
                    "label": "Review & Complete",
                    "type": "confirm"
                }
            ]
        })
    }
}
