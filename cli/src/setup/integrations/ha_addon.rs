use serde_json::{Value, json};

use super::IntegrationAdapter;

pub struct HaAddonAdapter {
    pub ha_url: Option<String>,
}

impl IntegrationAdapter for HaAddonAdapter {
    fn integration_type(&self) -> &str {
        "ha-addon"
    }

    fn display_name(&self) -> &str {
        "Home Assistant NyxID Add-on"
    }

    fn config(&self) -> Value {
        json!({
            "type": "ha-addon",
            "name": "Home Assistant NyxID Add-on",
            "steps": [
                {
                    "id": "create_key",
                    "label": "Create NyxID API Key",
                    "type": "auto",
                    "action": {
                        "method": "POST",
                        "path": "/api-keys",
                        "body": {
                            "name": "Home Assistant NyxID Add-on",
                            "platform": "generic"
                        }
                    }
                },
                {
                    "id": "show_credentials",
                    "label": "Add-on Credentials",
                    "type": "display",
                    "source_step": "create_key",
                    "instructions": "Copy these values into your Home Assistant NyxID Add-on configuration."
                },
                {
                    "id": "configure_ha",
                    "label": "Configure Home Assistant",
                    "type": "input",
                    "description": "Enter your Home Assistant details to verify the connection.",
                    "fields": [
                        {
                            "name": "ha_url",
                            "label": "Home Assistant URL",
                            "placeholder": "http://homeassistant.local:8123",
                            "value": self.ha_url.as_deref().unwrap_or(""),
                            "required": true
                        }
                    ]
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
