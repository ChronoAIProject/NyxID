//! Human capture and input are independent of cua agent sessions.
use super::{Runtime, native_desktop, string};
use anyhow::{Context, Result, bail};
use nyxid_machine::{
    Operation, Request,
    binary::{Frame, Kind},
    desktop::*,
};
use serde_json::{Value, json};
use std::{sync::Arc, time::Instant};
use tokio::sync::Mutex;
use uuid::Uuid;

pub struct Session {
    pub id: Uuid,
    pub refreshed: Instant,
    pub controller: Option<String>,
    pub control_revision: u64,
    pub owner_text: zeroize::Zeroizing<String>,
    pub frame_revision: u64,
    coordinates: Option<Coordinates>,
    pub budget: FrameBudget,
    sequence: u64,
    frame_sequence: u64,
}

#[cfg(test)]
pub(super) fn custody_test_session() -> Session {
    Session {
        id: Uuid::new_v4(),
        refreshed: Instant::now(),
        controller: None,
        control_revision: 0,
        owner_text: zeroize::Zeroizing::new(String::new()),
        frame_revision: 0,
        coordinates: None,
        budget: FrameBudget::default(),
        sequence: 0,
        frame_sequence: 0,
    }
}

#[derive(Default)]
pub struct Desktop {
    pub session: Mutex<Option<Session>>,
    pub sender: Mutex<Option<tokio::sync::mpsc::Sender<crate::node::ws_client::NodeWsMessage>>>,
    pub capture: Mutex<Option<tokio::task::JoinHandle<()>>>,
    #[cfg(target_os = "linux")]
    input: Arc<std::sync::Mutex<Option<native_desktop::Input>>>,
}
struct CaptureState {
    capture: native_desktop::Capture,
    encoder: native_desktop::Encoder,
    session: Uuid,
    revision: u64,
}
impl Runtime {
    pub(super) async fn desktop_activity(&self, tool: &str, display: Display) {
        let desktop = self.desktop_for(display);
        let mut session = desktop.session.lock().await;
        let Some(active) = session.as_mut().filter(|s| s.controller.is_none()) else {
            return;
        };
        // The native capture includes the cursor. No cua I/O or arguments enter
        // the metadata channel; takeover cannot wait for this notification.
        let Ok(bytes) = serde_json::to_vec(&json!({"tool":tool})) else {
            return;
        };
        active.sequence += 1;
        active.refreshed = Instant::now();
        let frame = Frame {
            kind: Kind::DesktopActivity,
            end: false,
            id: active.id,
            sequence: active.sequence,
            bytes: &bytes,
        }
        .encode();
        drop(session);
        if let Ok(frame) = frame
            && let Some(sender) = desktop.sender.lock().await.as_ref()
        {
            let _ = sender.try_send(crate::node::ws_client::NodeWsMessage::Binary(frame));
        }
    }

    pub(super) async fn desktop_open(&self, parameters: &Value) -> Result<Value> {
        let id = Uuid::parse_str(string(parameters, "session_id")?)?;
        let display = Display::from_parameters(parameters).map_err(anyhow::Error::msg)?;
        if display == Display::Secure {
            self.ensure_browser().await?;
        } else {
            self.ensure_dev_browser().await?;
        }
        let desktop = self.desktop_for(display);
        let mut session = desktop.session.lock().await;
        if session
            .as_ref()
            .is_some_and(|s| s.controller.is_none() && s.refreshed.elapsed() > IDLE_TIMEOUT)
        {
            *session = None;
        }
        if let Some(active) = session.as_mut() {
            if active.id != id {
                bail!("desktop session already open");
            }
            active.refreshed = Instant::now();
            if parameters["refresh_frame"] == true {
                active.frame_revision += 1;
                active.budget.reset();
            }
        } else {
            *session = Some(Session {
                id,
                refreshed: Instant::now(),
                controller: None,
                control_revision: 0,
                owner_text: zeroize::Zeroizing::new(String::new()),
                frame_revision: 0,
                coordinates: None,
                budget: FrameBudget::default(),
                sequence: 0,
                frame_sequence: 0,
            });
        }
        Ok(json!({"session_id":id,"streaming":true}))
    }

    pub(super) async fn desktop_input(&self, parameters: &Value) -> Result<Value> {
        let display = Display::from_parameters(parameters).map_err(anyhow::Error::msg)?;
        let desktop = self.desktop_for(display);
        let mut session = desktop.session.lock().await;
        let active = session.as_mut().context("desktop session closed")?;
        if parameters["session_id"] != active.id.to_string()
            || active.controller.as_deref() != parameters["viewer_id"].as_str()
            || parameters["revision"].as_u64() != Some(active.control_revision)
            || active.controller.is_none()
        {
            bail!("owner controller required");
        }
        let tool = string(parameters, "tool")?.to_owned();
        if !matches!(
            tool.as_str(),
            "move_cursor" | "click" | "drag" | "scroll" | "type_text" | "press_key" | "hotkey"
        ) {
            bail!("unsupported desktop input");
        }
        let mut args = parameters["arguments"].clone();
        if !args.is_object() || args.to_string().len() > 16384 {
            bail!("desktop input limit exceeded");
        }
        if matches!(tool.as_str(), "move_cursor" | "click" | "drag" | "scroll") {
            active
                .coordinates
                .as_ref()
                .context("wait for the first desktop frame")?
                .translate(&mut args)?;
        }
        let mut text = None;
        if tool == "type_text" {
            let value = string(&args, "text")?;
            if active.owner_text.len() + value.len() > 16384 {
                bail!("owner input field limit exceeded");
            }
            active.owner_text.push_str(value);
        } else if tool == "press_key" && args["key"] == "BACKSPACE" {
            active.owner_text.pop();
        } else if matches!(tool.as_str(), "click" | "drag" | "press_key" | "hotkey") {
            text = Some(std::mem::take(&mut active.owner_text));
        }
        active.refreshed = Instant::now();
        let control = self.control_for(display).subscribe();
        let revision = *control.borrow();
        drop(session);
        if let Some(text) = text.filter(|text| !text.is_empty()) {
            self.redactor
                .lock()
                .await
                .register(&text)
                .map_err(anyhow::Error::msg)?;
        }
        #[cfg(target_os = "linux")]
        {
            let input = desktop.input.clone();
            let endpoint = self.display_endpoint(display);
            tokio::task::spawn_blocking(move || -> Result<()> {
                let mut input = input
                    .lock()
                    .map_err(|_| anyhow::anyhow!("owner input stopped"))?;
                if *control.borrow() != revision {
                    bail!("desktop controller changed");
                }
                if input.is_none() {
                    *input = Some(native_desktop::Input::for_display(display, endpoint)?);
                }
                input
                    .as_mut()
                    .context("owner input unavailable")?
                    .send(&tool, &args, control, revision)
            })
            .await??;
        }
        #[cfg(target_os = "macos")]
        {
            let mut control = control;
            args["session"] = json!("nyxid-owner");
            args["target"] = json!({"kind":"desktop","display_id":"primary"});
            if tool == "click" {
                args["delivery_mode"] = json!("foreground");
            }
            let result = tokio::select! {
                biased;
                _=control.changed()=>bail!("desktop controller changed"),
                result=self.owner_driver.as_ref().context("cua unavailable")?.call(&tool,args)=>result?,
            };
            if *control.borrow() != revision || result["isError"] == true {
                bail!("owner input refused");
            }
        }
        Ok(json!({"accepted":true}))
    }

    pub(super) async fn start_capture(self: &Arc<Self>, display: Display) {
        let mut capture = self.desktop_for(display).capture.lock().await;
        if capture.as_ref().is_some_and(|task| !task.is_finished()) {
            return;
        }
        let weak = Arc::downgrade(self);
        *capture = Some(tokio::spawn(async move {
            let state = Arc::new(std::sync::Mutex::new(None));
            let mut interval = tokio::time::interval(FRAME_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut last_error = Instant::now() - std::time::Duration::from_secs(60);
            let screen_name = match display {
                Display::Secure => "secure",
                Display::Dev => "dev",
            };
            loop {
                interval.tick().await;
                let Some(runtime) = weak.upgrade() else {
                    break;
                };
                if let Err(error) = runtime.capture_once(display, state.clone()).await
                    && last_error.elapsed() >= std::time::Duration::from_secs(1)
                {
                    tracing::warn!(
                        screen = %screen_name,
                        error = %error,
                        "desktop capture failed"
                    );
                    last_error = Instant::now();
                }
            }
        }));
    }

    async fn capture_once(
        &self,
        display: Display,
        state: Arc<std::sync::Mutex<Option<CaptureState>>>,
    ) -> Result<()> {
        let screen_name = match display {
            Display::Secure => "secure",
            Display::Dev => "dev",
        };
        let desktop = self.desktop_for(display);
        let snapshot = {
            let mut session = desktop.session.lock().await;
            if let Some(active) = session.as_ref()
                && active.refreshed.elapsed() > IDLE_TIMEOUT
                && active.controller.is_none()
            {
                *session = None;
            }
            session
                .as_ref()
                .filter(|s| s.refreshed.elapsed() <= IDLE_TIMEOUT)
                .map(|s| {
                    (
                        s.id,
                        s.frame_revision,
                        s.frame_sequence,
                        *self.control_for(display).borrow(),
                    )
                })
        };
        let Some((id, revision, base, control)) = snapshot else {
            tokio::task::spawn_blocking(move || {
                if let Ok(mut state) = state.lock() {
                    state.take();
                }
            })
            .await?;
            return Ok(());
        };
        let sender = desktop
            .sender
            .lock()
            .await
            .clone()
            .context("desktop offline")?;
        if sender.capacity() < 4 {
            return Ok(());
        }
        // Neither authority nor session locks are held across capture/encoding.
        #[cfg(target_os = "linux")]
        let endpoint = self.display_endpoint(display);
        let encoded = tokio::task::spawn_blocking(move || -> Result<_> {
            let mut state = state
                .lock()
                .map_err(|_| anyhow::anyhow!("capture stopped"))?;
            if state.is_none() {
                *state = Some(CaptureState {
                    capture: native_desktop::Capture::for_display(
                        display,
                        #[cfg(target_os = "linux")]
                        endpoint,
                    )?,
                    encoder: native_desktop::Encoder::new(),
                    session: id,
                    revision,
                });
            }
            let state = state.as_mut().context("capture unavailable")?;
            let reset = state.session != id || state.revision != revision;
            state.session = id;
            state.revision = revision;
            let Some(pixels) = state.capture.capture()? else {
                return Ok(None);
            };
            state.encoder.encode(pixels, base, reset)
        })
        .await??;
        let Some((bytes, screen)) = encoded else {
            return Ok(());
        };
        let mut session = desktop.session.lock().await;
        let Some(active) = session.as_mut().filter(|s| s.id == id) else {
            return Ok(());
        };
        if *self.control_for(display).borrow() != control || active.frame_revision != revision {
            active.frame_revision += 1;
            return Ok(());
        }
        if !active.budget.admit(&bytes, bytes.len(), Instant::now()) {
            active.frame_revision += 1;
            return Ok(());
        }
        active.sequence += 1;
        let frame = Frame {
            kind: Kind::Desktop,
            end: false,
            id,
            sequence: active.sequence,
            bytes: &bytes,
        }
        .encode()
        .map_err(anyhow::Error::msg)?;
        if sender
            .try_send(crate::node::ws_client::NodeWsMessage::Binary(frame))
            .is_err()
        {
            active.frame_revision += 1;
            active.budget.reset();
        } else {
            active.frame_sequence = active.sequence;
            if active.frame_sequence == 1 {
                tracing::info!(
                    screen = %screen_name,
                    session_id = %id,
                    frame_bytes = bytes.len(),
                    "desktop first frame queued"
                );
            }
            active.coordinates = Some(Coordinates {
                image: [
                    f64::from(u16::from_be_bytes(bytes[4..6].try_into()?)),
                    f64::from(u16::from_be_bytes(bytes[6..8].try_into()?)),
                ],
                screen,
            });
        }
        Ok(())
    }

    pub(super) async fn input_frame(self: &Arc<Self>, frame: Frame<'_>) {
        if frame.kind != Kind::Input {
            return;
        }
        let Ok(request) = serde_json::from_slice::<Request>(frame.bytes) else {
            return;
        };
        if request.operation != Operation::DesktopInput
            || request.parameters["session_id"] != frame.id.to_string()
        {
            return;
        }
        let secret = self.desktop_secret.lock().await.clone();
        if let Some(secret) = secret {
            let _ = self.handle(request, &secret).await;
        }
    }
}

/// The browser sends capture pixels; cua accepts display points on Retina and
/// display pixels on Linux. The capture's own metadata is the authority.
struct Coordinates {
    image: [f64; 2],
    screen: [f64; 2],
}
impl Coordinates {
    fn translate(&self, arguments: &mut Value) -> Result<()> {
        for (key, axis) in [
            ("x", 0),
            ("y", 1),
            ("from_x", 0),
            ("from_y", 1),
            ("to_x", 0),
            ("to_y", 1),
        ] {
            if let Some(value) = arguments.get_mut(key) {
                let point = value
                    .as_f64()
                    .filter(|n| n.is_finite() && *n >= 0.0 && *n < self.image[axis])
                    .context("desktop coordinate outside frame")?;
                *value = json!(point * self.screen[axis] / self.image[axis]);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime(root: &std::path::Path) -> Arc<Runtime> {
        Runtime::new(
            &nyxid_machine::config::Config {
                roots: vec![root.to_owned()],
                allow_root: true,
                ..Default::default()
            },
            "node",
            &root.join("node"),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn capture_expires_idle_sessions_but_preserves_owner_control_and_backpressure() {
        let root = tempfile::tempdir().unwrap();
        let runtime = runtime(root.path());
        for display in [Display::Secure, Display::Dev] {
            let state = Arc::new(std::sync::Mutex::new(None));
            let desktop = runtime.desktop_for(display);
            runtime.capture_once(display, state.clone()).await.unwrap();

            let mut expired = custody_test_session();
            expired.refreshed = Instant::now() - IDLE_TIMEOUT - std::time::Duration::from_secs(1);
            *desktop.session.lock().await = Some(expired);
            runtime.capture_once(display, state.clone()).await.unwrap();
            assert!(desktop.session.lock().await.is_none());

            let mut controlled = custody_test_session();
            let id = controlled.id;
            controlled.controller = Some("owner".into());
            controlled.refreshed =
                Instant::now() - IDLE_TIMEOUT - std::time::Duration::from_secs(1);
            *desktop.session.lock().await = Some(controlled);
            runtime.capture_once(display, state.clone()).await.unwrap();
            assert_eq!(desktop.session.lock().await.as_ref().unwrap().id, id);

            desktop.session.lock().await.as_mut().unwrap().refreshed = Instant::now();
            assert_eq!(
                runtime
                    .capture_once(display, state.clone())
                    .await
                    .unwrap_err()
                    .to_string(),
                "desktop offline"
            );
            // A congested socket must not open the native display or encode a
            // frame. This exercises the real capture admission without X11.
            let (sender, mut receiver) = tokio::sync::mpsc::channel(3);
            *desktop.sender.lock().await = Some(sender);
            runtime.capture_once(display, state.clone()).await.unwrap();
            assert!(state.lock().unwrap().is_none());
            assert!(matches!(
                receiver.try_recv(),
                Err(tokio::sync::mpsc::error::TryRecvError::Empty)
            ));
            let session = desktop.session.lock().await;
            let active = session.as_ref().unwrap();
            assert_eq!(active.id, id);
            assert_eq!(active.controller.as_deref(), Some("owner"));
            assert_eq!((active.sequence, active.frame_sequence), (0, 0));
        }
    }

    #[tokio::test]
    async fn activity_frames_stay_on_the_selected_display_and_pause_for_owner() {
        use crate::node::ws_client::NodeWsMessage;
        let root = tempfile::tempdir().unwrap();
        let runtime = runtime(root.path());
        let mut receivers = Vec::new();
        for display in [Display::Secure, Display::Dev] {
            let desktop = runtime.desktop_for(display);
            let (sender, receiver) = tokio::sync::mpsc::channel(4);
            *desktop.sender.lock().await = Some(sender);
            runtime.desktop_activity("no_session", display).await;
            let session = custody_test_session();
            let id = session.id;
            *desktop.session.lock().await = Some(session);
            receivers.push((display, id, receiver));
        }
        runtime.desktop_activity("click", Display::Secure).await;
        runtime.desktop_activity("scroll", Display::Secure).await;
        runtime.desktop_activity("navigate", Display::Dev).await;

        for (display, id, mut receiver) in receivers {
            let tools: &[&str] = match display {
                Display::Secure => &["click", "scroll"],
                Display::Dev => &["navigate"],
            };
            for (index, tool) in tools.iter().enumerate() {
                let NodeWsMessage::Binary(bytes) = receiver.try_recv().unwrap() else {
                    panic!("activity must use a binary machine frame");
                };
                let frame = Frame::decode(&bytes).unwrap();
                assert_eq!(frame.kind, Kind::DesktopActivity);
                assert_eq!(frame.id, id);
                assert_eq!(frame.sequence, index as u64 + 1);
                assert!(!frame.end);
                assert_eq!(
                    serde_json::from_slice::<Value>(frame.bytes).unwrap(),
                    json!({"tool":tool})
                );
            }
            let desktop = runtime.desktop_for(display);
            desktop.session.lock().await.as_mut().unwrap().controller = Some("owner".into());
            runtime.desktop_activity("owner_is_typing", display).await;
            assert!(matches!(
                receiver.try_recv(),
                Err(tokio::sync::mpsc::error::TryRecvError::Empty)
            ));
            assert_eq!(
                desktop.session.lock().await.as_ref().unwrap().sequence,
                tools.len() as u64
            );
        }
    }

    #[tokio::test]
    async fn desktop_input_rejects_stale_controller_and_invalid_payload_before_dispatch() {
        let root = tempfile::tempdir().unwrap();
        let runtime = runtime(root.path());
        let mut session = custody_test_session();
        let id = session.id;
        session.controller = Some("owner".into());
        session.control_revision = 7;
        *runtime.desktop.session.lock().await = Some(session);
        let valid = json!({"session_id":id,"viewer_id":"owner","revision":7,
            "tool":"type_text","arguments":{"text":"x"}});
        for (key, value, reason) in [
            (
                "session_id",
                json!(Uuid::new_v4()),
                "owner controller required",
            ),
            ("viewer_id", json!("old-owner"), "owner controller required"),
            ("revision", json!(6), "owner controller required"),
            ("revision", Value::Null, "owner controller required"),
            ("display", json!("dev"), "desktop session closed"),
            ("tool", json!("run_command"), "unsupported desktop input"),
            ("arguments", json!([]), "desktop input limit exceeded"),
            (
                "arguments",
                json!({"text":"x".repeat(16385)}),
                "desktop input limit exceeded",
            ),
        ] {
            let mut input = valid.clone();
            input[key] = value;
            assert_eq!(
                runtime.desktop_input(&input).await.unwrap_err().to_string(),
                reason
            );
            assert!(
                runtime
                    .desktop
                    .session
                    .lock()
                    .await
                    .as_ref()
                    .unwrap()
                    .owner_text
                    .is_empty()
            );
        }
        let mut click = valid.clone();
        click["tool"] = json!("click");
        click["arguments"] = json!({"x":640,"y":400});
        assert_eq!(
            runtime.desktop_input(&click).await.unwrap_err().to_string(),
            "wait for the first desktop frame"
        );
        runtime
            .desktop
            .session
            .lock()
            .await
            .as_mut()
            .unwrap()
            .coordinates = Some(Coordinates {
            image: [640.0, 400.0],
            screen: [1280.0, 800.0],
        });
        assert_eq!(
            runtime.desktop_input(&click).await.unwrap_err().to_string(),
            "desktop coordinate outside frame"
        );
        runtime
            .desktop
            .session
            .lock()
            .await
            .as_mut()
            .unwrap()
            .owner_text
            .push_str(&"x".repeat(16384));
        assert_eq!(
            runtime.desktop_input(&valid).await.unwrap_err().to_string(),
            "owner input field limit exceeded"
        );
        runtime
            .desktop
            .session
            .lock()
            .await
            .as_mut()
            .unwrap()
            .controller = None;
        assert_eq!(
            runtime.desktop_input(&valid).await.unwrap_err().to_string(),
            "owner controller required"
        );
        runtime.desktop.session.lock().await.take();
        assert_eq!(
            runtime.desktop_input(&valid).await.unwrap_err().to_string(),
            "desktop session closed"
        );
    }

    #[tokio::test]
    async fn capture_resumes_after_shutdown_and_preserves_running_task_on_reconnect() {
        let root = tempfile::tempdir().unwrap();
        let runtime = Runtime::new(
            &nyxid_machine::config::Config {
                roots: vec![root.path().to_owned()],
                allow_root: true,
                ..Default::default()
            },
            "node",
            &root.path().join("node"),
        )
        .unwrap();
        let mut original = Vec::new();
        for display in [Display::Secure, Display::Dev] {
            runtime.start_capture(display).await;
            original.push(
                runtime
                    .desktop_for(display)
                    .capture
                    .lock()
                    .await
                    .as_ref()
                    .unwrap()
                    .abort_handle(),
            );
        }

        runtime.disconnect().await;
        for (display, task) in [Display::Secure, Display::Dev].into_iter().zip(&original) {
            runtime.start_capture(display).await;
            assert_eq!(
                runtime
                    .desktop_for(display)
                    .capture
                    .lock()
                    .await
                    .as_ref()
                    .unwrap()
                    .id(),
                task.id()
            );
            assert!(!task.is_finished());
        }

        runtime.shutdown().await;
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while original.iter().any(|task| !task.is_finished()) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        for (display, previous) in [Display::Secure, Display::Dev].into_iter().zip(original) {
            runtime.start_capture(display).await;
            let capture = runtime.desktop_for(display).capture.lock().await;
            let task = capture.as_ref().unwrap();
            assert!(
                !task.is_finished(),
                "reconnected desktop must have a live capture task"
            );
            assert_ne!(task.id(), previous.id());
        }
        runtime.shutdown().await;
    }

    #[test]
    fn input_maps_downscaled_retina_frames_to_display_points() {
        let coordinates = Coordinates {
            image: [1280.0, 800.0],
            screen: [2560.0, 1600.0],
        };
        let mut input =
            json!({"x":640,"y":400,"from_x":0,"from_y":100,"to_x":1200,"to_y":700,"amount":3});
        coordinates.translate(&mut input).unwrap();
        assert_eq!(
            input,
            json!({"x":1280.0,"y":800.0,"from_x":0.0,"from_y":200.0,"to_x":2400.0,"to_y":1400.0,"amount":3})
        );
        for point in [-1.0, 1280.0, 1e30] {
            assert!(
                coordinates
                    .translate(&mut json!({"x":point,"y":1}))
                    .is_err()
            );
        }
    }
}
