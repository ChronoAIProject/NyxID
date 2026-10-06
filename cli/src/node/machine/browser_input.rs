//! Secure-browser hit testing is extension-owned; actual input is OS-owned.
use super::{MachineError, Runtime, cancellation};
use anyhow::{Context, Result, bail};
use nyxid_machine::desktop::Display;
use serde_json::{Value, json};

impl Runtime {
    pub(super) async fn secure_browser_action(&self, parameters: &Value) -> Result<Value> {
        let browser = self.browser.lock().await;
        let browser = browser.as_ref().context(MachineError::Browser)?;
        let action = parameters["action"]
            .as_str()
            .context("browser action required")?;
        if !matches!(action, "click" | "type" | "select" | "press")
            || parameters["input_mode"] == "dom_fallback"
        {
            return browser.action(parameters.clone()).await;
        }
        let mut probe = parameters.clone();
        probe["action"] = json!("_prepare");
        probe["input_action"] = json!(action);
        let prepared = browser.action(probe).await?;
        if prepared["status"] != "ready" {
            return Ok(prepared);
        }
        self.browser_native_input(parameters, &prepared, Display::Secure)
            .await?;
        tokio::time::sleep(std::time::Duration::from_millis(35)).await;
        let mut observation = parameters.clone();
        observation["action"] = json!("snapshot");
        observation["tab_id"] = prepared["tab_id"].clone();
        let mut result = browser.action(observation).await?;
        result["input_mode"] = json!("trusted");
        Ok(result)
    }
    pub(super) async fn browser_native_input(
        &self,
        parameters: &Value,
        prepared: &Value,
        display: Display,
    ) -> Result<()> {
        let action = parameters["action"]
            .as_str()
            .context("browser action required")?;
        let point = &prepared["point"];
        let n = |key: &str| {
            point[key]
                .as_f64()
                .filter(|n| n.is_finite())
                .context("invalid browser coordinates")
        };
        let scale = if cfg!(target_os = "linux") {
            n("dpr")?
        } else {
            1.0
        };
        let x = (n("screen_x")? + n("ui_x")? + n("x")?) * scale;
        let y = (n("screen_y")? + n("ui_y")? + n("y")?) * scale;
        if !(0.0..7680.0).contains(&x) || !(0.0..4320.0).contains(&y) {
            bail!("browser outside display");
        }
        let mut steps = Vec::new();
        // Typing/pressing use the confirmed focused element. An extra click can
        // race a newly navigated iframe compositor or activate a button twice.
        if !matches!(action, "press" | "type") || prepared["focused"] != true {
            steps.push(("click", json!({"x":x,"y":y})));
        }
        match action {
            "type" => {
                steps.push((
                    "hotkey",
                    json!({"keys":[if cfg!(target_os="macos"){"CMD"}else{"CTRL"},"A"]}),
                ));
                steps.push(("type_text", json!({"text":parameters["text"]})));
            }
            "press" => steps.push((
                "press_key",
                json!({"key":key(parameters["key"].as_str().unwrap_or_default())}),
            )),
            "select" => {
                let option = prepared["option"]
                    .as_u64()
                    .filter(|n| *n <= 10000)
                    .context("invalid option")?;
                steps.push(("press_key", json!({"key":"HOME"})));
                for _ in 0..option {
                    steps.push(("press_key", json!({"key":"DOWN"})));
                }
                steps.push(("press_key", json!({"key":"ENTER"})));
            }
            _ => {}
        }
        let scope: cancellation::Scope = serde_json::from_value(parameters.clone())?;
        let stopped = self.turns.subscribe(&scope);
        let control = self.control_for(display).subscribe();
        let revision = *control.borrow();
        if revision & 1 != 0 || *stopped.borrow() {
            bail!("browser input cancelled");
        }
        #[cfg(target_os = "linux")]
        let endpoint = self.display_endpoint(display);
        #[cfg(target_os = "linux")]
        tokio::task::spawn_blocking(move || -> Result<()> {
            // Dedicated input connection; owner input never queues behind it.
            let mut input = super::native_desktop::Input::for_display(display, endpoint)?;
            input.stop_when(Some(stopped));
            for (tool, args) in steps {
                input.send(tool, &args, control.clone(), revision)?;
                // XTest acknowledgement means the X server queued the event;
                // give Chromium time to focus/open native popups before the next
                // key step. Never hold the controller lock across this wait.
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Ok(())
        })
        .await??;
        #[cfg(target_os = "macos")]
        for (tool, mut args) in steps {
            if *control.borrow() != revision || *stopped.borrow() {
                bail!("browser input cancelled");
            }
            args["target"] = json!({"kind":"desktop","display_id":"primary"});
            args["session"] = json!("nyxid-agent");
            let result = self
                .driver
                .as_ref()
                .context(MachineError::Browser)?
                .call(tool, args)
                .await?;
            if result["isError"] == true {
                bail!("trusted browser input refused; observe before retrying");
            }
        }
        Ok(())
    }
}
fn key(value: &str) -> String {
    value
        .strip_prefix("Arrow")
        .unwrap_or(value)
        .to_ascii_uppercase()
}
