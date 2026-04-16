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
