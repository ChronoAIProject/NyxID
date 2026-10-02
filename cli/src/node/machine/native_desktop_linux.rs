use super::{Pixels, rgb_from_bgra};
use anyhow::{Context, Result, bail};
use serde_json::Value;
use x11rb::{
    connection::Connection,
    protocol::{xfixes::ConnectionExt as _, xproto::*, xtest::ConnectionExt as _},
    rust_connection::RustConnection,
};

fn connect(display: nyxid_machine::desktop::Display) -> Result<(RustConnection, usize)> {
    if display == nyxid_machine::desktop::Display::Secure {
        return Ok(x11rb::connect(None)?);
    }
    let (name, authority) = crate::node::machine::dev_display::endpoint()?;
    let number: u16 = name
        .strip_prefix(':')
        .context("Local display required")?
        .parse()?;
    let auth = std::fs::read(authority)?;
    anyhow::ensure!(auth.len() <= 65536, "Xauthority limit exceeded");
    let mut bytes = auth.as_slice();
    while bytes.len() >= 2 {
        bytes = &bytes[2..]; // family
        let mut fields = Vec::new();
        for _ in 0..4 {
            anyhow::ensure!(bytes.len() >= 2, "Invalid Xauthority");
            let len = u16::from_be_bytes([bytes[0], bytes[1]]) as usize;
            anyhow::ensure!(bytes.len() >= len + 2, "Invalid Xauthority");
            fields.push(&bytes[2..2 + len]);
            bytes = &bytes[2 + len..];
        }
        if (fields[1].is_empty() || fields[1] == number.to_string().as_bytes())
            && fields[2] == b"MIT-MAGIC-COOKIE-1"
        {
            let stream =
                std::os::unix::net::UnixStream::connect(format!("/tmp/.X11-unix/X{number}"))?;
            let (stream, _) = x11rb::rust_connection::DefaultStream::from_unix_stream(stream)?;
            return Ok((
                RustConnection::connect_to_stream_with_auth_info(
                    stream,
                    0,
                    fields[2].to_vec(),
                    fields[3].to_vec(),
                )?,
                0,
            ));
        }
    }
    bail!("Developer display cookie unavailable")
}

pub struct Capture {
    connection: RustConnection,
    root: Window,
}
impl Capture {
    pub fn for_display(display: nyxid_machine::desktop::Display) -> Result<Self> {
        // The supervisor owns the browser's Xauthority; no DISPLAY/cookie is
        // passed to agent command children. Capture and input use separate X
        // connections, so an outstanding GetImage never blocks owner input.
        let (connection, screen) = connect(display)?;
        let root = connection.setup().roots[screen].root;
        connection.xfixes_query_version(5, 0)?.reply()?;
        Ok(Self { connection, root })
    }
    pub fn capture(&mut self) -> Result<Option<Pixels>> {
        let geometry = self.connection.get_geometry(self.root)?.reply()?;
        let width = u32::from(geometry.width);
        let height = u32::from(geometry.height);
        anyhow::ensure!(
            width <= 7680 && height <= 4320,
            "desktop dimensions exceed the 8K capture limit"
        );
        let frame = self
            .connection
            .get_image(
                ImageFormat::Z_PIXMAP,
                self.root,
                0,
                0,
                geometry.width,
                geometry.height,
                u32::MAX,
            )?
            .reply()?;
        let mut rgb = rgb_from_bgra(width, height, width as usize * 4, &frame.data)?;
        // XGetImage excludes the hardware cursor. Composite XFixes' premultiplied
        // cursor in memory so both human and agent motion remain visible.
        let cursor = self.connection.xfixes_get_cursor_image()?.reply()?;
        for cy in 0..i32::from(cursor.height) {
            for cx in 0..i32::from(cursor.width) {
                let x = i32::from(cursor.x) - i32::from(cursor.xhot) + cx;
                let y = i32::from(cursor.y) - i32::from(cursor.yhot) + cy;
                if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
                    continue;
                }
                let p = cursor.cursor_image[(cy * i32::from(cursor.width) + cx) as usize];
                let alpha = (p >> 24) & 255;
                let offset = (y as usize * width as usize + x as usize) * 3;
                for (channel, shift) in [16, 8, 0].into_iter().enumerate() {
                    rgb[offset + channel] = (((p >> shift) & 255)
                        + (u32::from(rgb[offset + channel]) * (255 - alpha) / 255))
                        .min(255) as u8;
                }
            }
        }
        Ok(Some(
            Pixels {
                width,
                height,
                screen: [f64::from(width), f64::from(height)],
                rgb,
            }
            .bounded(),
        ))
    }
}

pub struct Input {
    connection: RustConnection,
    root: Window,
    authority: Option<(tokio::sync::watch::Receiver<u64>, u64)>,
    stopped: Option<tokio::sync::watch::Receiver<bool>>,
}
impl Input {
    pub fn for_display(display: nyxid_machine::desktop::Display) -> Result<Self> {
        let (connection, screen) = connect(display)?;
        let root = connection.setup().roots[screen].root;
        connection.xtest_get_version(2, 2)?.reply()?;
        Ok(Self {
            connection,
            root,
            authority: None,
            stopped: None,
        })
    }
    pub fn stop_when(&mut self, stopped: Option<tokio::sync::watch::Receiver<bool>>) {
        self.stopped = stopped;
    }
    fn event(&self, kind: u8, detail: u8, x: i16, y: i16) -> Result<()> {
        if self.stopped.as_ref().is_some_and(|s| *s.borrow()) {
            bail!("machine turn stopped");
        }
        if self
            .authority
            .as_ref()
            .is_none_or(|(control, revision)| *control.borrow() != *revision)
        {
            bail!("desktop controller changed");
        }
        self.connection
            .xtest_fake_input(kind, detail, 0, self.root, x, y, 0)?
            .check()?;
        Ok(())
    }
    fn point(&self, args: &Value, x: &str, y: &str) -> Result<()> {
        let x = args[x].as_f64().context("missing pointer x")? as i16;
        let y = args[y].as_f64().context("missing pointer y")? as i16;
        self.event(MOTION_NOTIFY_EVENT, 0, x, y)
    }
    fn stroke(&self, code: u8) -> Result<()> {
        self.event(KEY_PRESS_EVENT, code, 0, 0)?;
        self.event(KEY_RELEASE_EVENT, code, 0, 0)
    }
    fn keycode(&self, key: &str) -> Result<u8> {
        let symbol = match key {
            "CTRL" | "CONTROL" => 0xffe3,
            "SHIFT" => 0xffe1,
            "ALT" => 0xffe9,
            "META" | "SUPER" => 0xffeb,
            "ENTER" | "RETURN" => 0xff0d,
            "TAB" => 0xff09,
            "ESC" | "ESCAPE" => 0xff1b,
            "BACKSPACE" => 0xff08,
            "DELETE" => 0xffff,
            "LEFT" => 0xff51,
            "UP" => 0xff52,
            "RIGHT" => 0xff53,
            "DOWN" => 0xff54,
            "HOME" => 0xff50,
            "END" => 0xff57,
            "PAGEUP" => 0xff55,
            "PAGEDOWN" => 0xff56,
            "SPACE" => 0x20,
            name if name.len() == 1 => u32::from(name.to_ascii_lowercase().as_bytes()[0]),
            name if name.starts_with('F') => {
                0xffbd
                    + name[1..]
                        .parse::<u32>()
                        .ok()
                        .filter(|v| (1..=24).contains(v))
                        .context("unsupported key")?
            }
            _ => bail!("unsupported key"),
        };
        let setup = self.connection.setup();
        let map = self
            .connection
            .get_keyboard_mapping(setup.min_keycode, setup.max_keycode - setup.min_keycode + 1)?
            .reply()?;
        map.keysyms
            .chunks(map.keysyms_per_keycode as usize)
            .position(|codes| codes.contains(&symbol))
            .map(|index| index as u8 + setup.min_keycode)
            .context("key unavailable")
    }
    pub fn send(
        &mut self,
        tool: &str,
        args: &Value,
        control: tokio::sync::watch::Receiver<u64>,
        revision: u64,
    ) -> Result<()> {
        self.authority = Some((control, revision));
        match tool {
            "move_cursor" => self.point(args, "x", "y")?,
            "click" | "drag" => {
                let button = match args["button"].as_str().unwrap_or("left") {
                    "left" => 1,
                    "middle" => 2,
                    "right" => 3,
                    _ => bail!("unsupported button"),
                };
                if tool == "drag" {
                    self.point(args, "from_x", "from_y")?;
                } else {
                    self.point(args, "x", "y")?;
                }
                for _ in 0..args["count"].as_u64().unwrap_or(1).clamp(1, 3) {
                    self.event(BUTTON_PRESS_EVENT, button, 0, 0)?;
                    if tool == "drag" {
                        self.point(args, "to_x", "to_y")?;
                    }
                    self.event(BUTTON_RELEASE_EVENT, button, 0, 0)?;
                }
            }
            "scroll" => {
                self.point(args, "x", "y")?;
                let button = match args["direction"].as_str().unwrap_or("down") {
                    "up" => 4,
                    "down" => 5,
                    "left" => 6,
                    "right" => 7,
                    _ => bail!("unsupported direction"),
                };
                for _ in 0..args["amount"].as_u64().unwrap_or(3).clamp(1, 100) {
                    self.event(BUTTON_PRESS_EVENT, button, 0, 0)?;
                    self.event(BUTTON_RELEASE_EVENT, button, 0, 0)?;
                }
            }
            "press_key" => {
                self.stroke(self.keycode(args["key"].as_str().context("missing key")?)?)?
            }
            "hotkey" => {
                let names = args["keys"]
                    .as_array()
                    .filter(|v| v.len() <= 8)
                    .context("invalid hotkey")?;
                let codes = names
                    .iter()
                    .map(|name| self.keycode(name.as_str().context("invalid key")?))
                    .collect::<Result<Vec<_>>>()?;
                let mut pressed = Vec::new();
                let result = (|| -> Result<()> {
                    for code in &codes {
                        self.event(KEY_PRESS_EVENT, *code, 0, 0)?;
                        pressed.push(*code);
                    }
                    Ok(())
                })();
                for code in pressed.iter().rev() {
                    self.connection
                        .xtest_fake_input(KEY_RELEASE_EVENT, *code, 0, self.root, 0, 0, 0)?
                        .check()?;
                }
                result?;
            }
            "type_text" => {
                let text = args["text"]
                    .as_str()
                    .filter(|s| s.len() <= 8192)
                    .context("invalid text")?;
                // Use the real keyboard map for ordinary text. Rebinding a
                // single key between events races Chromium's map notifications.
                let setup = self.connection.setup();
                let map = self
                    .connection
                    .get_keyboard_mapping(
                        setup.min_keycode,
                        setup.max_keycode - setup.min_keycode + 1,
                    )?
                    .reply()?;
                let width = map.keysyms_per_keycode as usize;
                let shift = self.keycode("SHIFT")?;
                for ch in text.chars() {
                    let symbol = match ch {
                        '\n' => 0xff0d,
                        '\t' => 0xff09,
                        c if (c as u32) <= 255 => c as u32,
                        c => 0x01000000 | c as u32,
                    };
                    let mapped = map
                        .keysyms
                        .chunks(width)
                        .enumerate()
                        .find_map(|(index, syms)| {
                            syms.iter()
                                .take(2)
                                .position(|s| *s == symbol)
                                .map(|level| (setup.min_keycode + index as u8, level == 1))
                        });
                    if let Some((code, shifted)) = mapped {
                        if shifted {
                            self.event(KEY_PRESS_EVENT, shift, 0, 0)?;
                        }
                        let result = self.stroke(code);
                        if shifted {
                            // Release our modifier even when takeover cancels
                            // typing, so the human never inherits a stuck Shift.
                            self.connection
                                .xtest_fake_input(KEY_RELEASE_EVENT, shift, 0, self.root, 0, 0, 0)?
                                .check()?;
                        }
                        result?;
                    } else {
                        // Chromium on Linux supports Unicode entry through the
                        // input method only on some desktops. Use one stable
                        // temporary keysym with a delivery pause on each side.
                        let code = setup.max_keycode;
                        let old = self.connection.get_keyboard_mapping(code, 1)?.reply()?;
                        self.connection
                            .change_keyboard_mapping(1, code, 1, &[symbol])?
                            .check()?;
                        std::thread::sleep(std::time::Duration::from_millis(20));
                        let result = self.stroke(code);
                        std::thread::sleep(std::time::Duration::from_millis(20));
                        self.connection
                            .change_keyboard_mapping(
                                1,
                                code,
                                old.keysyms_per_keycode,
                                &old.keysyms,
                            )?
                            .check()?;
                        result?;
                    }
                }
            }
            _ => bail!("unsupported owner input"),
        }
        self.connection.flush()?;
        Ok(())
    }
}
