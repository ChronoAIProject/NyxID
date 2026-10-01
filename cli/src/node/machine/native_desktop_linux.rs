use super::{Pixels, rgb_from_bgra};
use anyhow::{Context, Result, bail};
use serde_json::Value;
use x11rb::{
    connection::Connection,
    protocol::{xfixes::ConnectionExt as _, xproto::*, xtest::ConnectionExt as _},
    rust_connection::RustConnection,
};

pub struct Capture {
    connection: RustConnection,
    root: Window,
}
impl Capture {
    pub fn new() -> Result<Self> {
        // The supervisor owns the browser's Xauthority; no DISPLAY/cookie is
        // passed to agent command children. Capture and input use separate X
        // connections, so an outstanding GetImage never blocks owner input.
        let (connection, screen) = x11rb::connect(None)?;
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
}
impl Input {
    pub fn new() -> Result<Self> {
        let (connection, screen) = x11rb::connect(None)?;
        let root = connection.setup().roots[screen].root;
        connection.xtest_get_version(2, 2)?.reply()?;
        Ok(Self {
            connection,
            root,
            authority: None,
        })
    }
    fn event(&self, kind: u8, detail: u8, x: i16, y: i16) -> Result<()> {
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
                for code in &codes {
                    self.event(KEY_PRESS_EVENT, *code, 0, 0)?;
                }
                for code in codes.iter().rev() {
                    self.event(KEY_RELEASE_EVENT, *code, 0, 0)?;
                }
            }
            "type_text" => {
                let text = args["text"]
                    .as_str()
                    .filter(|s| s.len() <= 8192)
                    .context("invalid text")?;
                // Unicode keysyms produce real keyboard events without clipboard
                // or disk writes. Restore the spare keycode even on failure.
                let code = self.connection.setup().max_keycode;
                let old = self.connection.get_keyboard_mapping(code, 1)?.reply()?;
                let result = (|| -> Result<()> {
                    for ch in text.chars() {
                        let symbol = match ch {
                            '\n' => 0xff0d,
                            '\t' => 0xff09,
                            c if (c as u32) <= 255 => c as u32,
                            c => 0x01000000 | c as u32,
                        };
                        self.connection
                            .change_keyboard_mapping(1, code, 1, &[symbol])?
                            .check()?;
                        self.stroke(code)?;
                    }
                    Ok(())
                })();
                self.connection
                    .change_keyboard_mapping(1, code, old.keysyms_per_keycode, &old.keysyms)?
                    .check()?;
                result?;
            }
            _ => bail!("unsupported owner input"),
        }
        self.connection.flush()?;
        Ok(())
    }
}
