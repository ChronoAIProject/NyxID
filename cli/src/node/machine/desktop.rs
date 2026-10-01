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
#[derive(Default)]
pub struct Desktop {
    pub session: Mutex<Option<Session>>,
    pub sender: Mutex<Option<tokio::sync::mpsc::Sender<crate::node::ws_client::NodeWsMessage>>>,
    pub capture: std::sync::OnceLock<tokio::task::JoinHandle<()>>,
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
    pub(super) async fn desktop_activity(&self, tool: &str) {
        let mut session = self.desktop.session.lock().await;
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
            && let Some(sender) = self.desktop.sender.lock().await.as_ref()
        {
            let _ = sender.try_send(crate::node::ws_client::NodeWsMessage::Binary(frame));
        }
    }

    pub(super) async fn desktop_open(&self, parameters: &Value) -> Result<Value> {
        let id = Uuid::parse_str(string(parameters, "session_id")?)?;
        self.ensure_browser().await?;
        let mut session = self.desktop.session.lock().await;
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
        let mut session = self.desktop.session.lock().await;
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
        let control = self.owner_control.subscribe();
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
            let input = self.desktop.input.clone();
            tokio::task::spawn_blocking(move || -> Result<()> {
                let mut input = input
                    .lock()
                    .map_err(|_| anyhow::anyhow!("owner input stopped"))?;
                if *control.borrow() != revision {
                    bail!("desktop controller changed");
                }
                if input.is_none() {
                    *input = Some(native_desktop::Input::new()?);
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

    pub(super) fn start_capture(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        self.desktop.capture.get_or_init(|| {
            tokio::spawn(async move {
                let state = Arc::new(std::sync::Mutex::new(None));
                let mut interval = tokio::time::interval(FRAME_INTERVAL);
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    interval.tick().await;
                    let Some(runtime) = weak.upgrade() else {
                        break;
                    };
                    let _ = runtime.capture_once(state.clone()).await;
                }
            })
        });
    }

    async fn capture_once(&self, state: Arc<std::sync::Mutex<Option<CaptureState>>>) -> Result<()> {
        let snapshot = {
            let mut session = self.desktop.session.lock().await;
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
                        *self.owner_control.borrow(),
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
        let sender = self
            .desktop
            .sender
            .lock()
            .await
            .clone()
            .context("desktop offline")?;
        if sender.capacity() < 4 {
            return Ok(());
        }
        // Neither authority nor session locks are held across capture/encoding.
        let encoded = tokio::task::spawn_blocking(move || -> Result<_> {
            let mut state = state
                .lock()
                .map_err(|_| anyhow::anyhow!("capture stopped"))?;
            if state.is_none() {
                *state = Some(CaptureState {
                    capture: native_desktop::Capture::new()?,
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
        let mut session = self.desktop.session.lock().await;
        let Some(active) = session.as_mut().filter(|s| s.id == id) else {
            return Ok(());
        };
        if *self.owner_control.borrow() != control || active.frame_revision != revision {
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
