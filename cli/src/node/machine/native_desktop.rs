//! Human-only capture: raw pixels never leave this in-memory pipeline except
//! through an authenticated desktop session. Agent observations still use cua.
use anyhow::{Context, Result};
use zeroize::Zeroizing;

#[derive(Clone)]
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub screen: [f64; 2],
    pub rgb: Zeroizing<Vec<u8>>,
}

#[cfg(any(target_os = "linux", test))]
impl Pixels {
    fn bounded(self) -> Self {
        let scale = (1920.0 / f64::from(self.width))
            .min(1200.0 / f64::from(self.height))
            .min(1.0);
        if scale == 1.0 {
            return self;
        }
        let width = (f64::from(self.width) * scale) as u32;
        let height = (f64::from(self.height) * scale) as u32;
        let mut rgb = Zeroizing::new(Vec::with_capacity(width as usize * height as usize * 3));
        for y in 0..height {
            for x in 0..width {
                let offset = ((y * self.height / height) as usize * self.width as usize
                    + (x * self.width / width) as usize)
                    * 3;
                rgb.extend_from_slice(&self.rgb[offset..offset + 3]);
            }
        }
        Self {
            width,
            height,
            screen: self.screen,
            rgb,
        }
    }
}

#[cfg(target_os = "linux")]
#[path = "native_desktop_linux.rs"]
mod platform;
#[cfg(target_os = "macos")]
#[path = "native_desktop_macos.rs"]
mod platform;
pub use platform::Capture;
#[cfg(target_os = "linux")]
pub use platform::Input;

/// A JPEG dirty rectangle. The fixed header binds it to the previously sent
/// desktop sequence; a missing base forces a keyframe instead of stale pixels.
pub struct Encoder {
    previous: Option<Pixels>,
}
impl Encoder {
    pub fn new() -> Self {
        Self { previous: None }
    }
    pub fn encode(
        &mut self,
        pixels: Pixels,
        base: u64,
        reset: bool,
    ) -> Result<Option<(Vec<u8>, [f64; 2])>> {
        let width = pixels.width as usize;
        let height = pixels.height as usize;
        let full = reset
            || self
                .previous
                .as_ref()
                .is_none_or(|old| old.width != pixels.width || old.height != pixels.height);
        let (mut left, mut top, mut right, mut bottom) = (width, height, 0, 0);
        if full {
            (left, top, right, bottom) = (0, 0, width, height);
        } else if let Some(old) = &self.previous {
            // Compare 64-pixel tiles with memcmp; merge the dirty tiles into a
            // rectangle. Text edits usually encode only one small band.
            for y in (0..height).step_by(64) {
                for x in (0..width).step_by(64) {
                    let xend = (x + 64).min(width);
                    let yend = (y + 64).min(height);
                    if (y..yend).any(|row| {
                        let span = row * width * 3 + x * 3..row * width * 3 + xend * 3;
                        pixels.rgb[span.clone()] != old.rgb[span]
                    }) {
                        left = left.min(x);
                        top = top.min(y);
                        right = right.max(xend);
                        bottom = bottom.max(yend);
                    }
                }
            }
        }
        if right == 0 {
            return Ok(None);
        }
        let mut rectangle = Zeroizing::new(Vec::with_capacity((right - left) * (bottom - top) * 3));
        for y in top..bottom {
            rectangle.extend_from_slice(
                &pixels.rgb[y * width * 3 + left * 3..y * width * 3 + right * 3],
            );
        }
        let mut encoded = Vec::with_capacity(65536);
        encoded.extend_from_slice(b"NYXD");
        for n in [width, height, left, top, right - left, bottom - top] {
            encoded.extend_from_slice(&u16::try_from(n)?.to_be_bytes());
        }
        encoded.extend_from_slice(&(if full { 0 } else { base }).to_be_bytes());
        jpeg_encoder::Encoder::new(&mut encoded, 70).encode(
            &rectangle,
            (right - left) as u16,
            (bottom - top) as u16,
            jpeg_encoder::ColorType::Rgb,
        )?;
        let screen = pixels.screen;
        self.previous = Some(pixels);
        Ok(Some((encoded, screen)))
    }
}

fn rgb_from_bgra(
    width: u32,
    height: u32,
    stride: usize,
    bytes: &[u8],
) -> Result<Zeroizing<Vec<u8>>> {
    anyhow::ensure!(
        width > 0 && height > 0 && width <= 7680 && height <= 4320,
        "desktop dimensions exceed limit"
    );
    anyhow::ensure!(stride >= width as usize * 4, "invalid pixel stride");
    let mut rgb = Zeroizing::new(Vec::with_capacity(width as usize * height as usize * 3));
    for y in 0..height as usize {
        let row = bytes
            .get(y * stride..y * stride + width as usize * 4)
            .context("invalid pixel buffer")?;
        for pixel in row.as_chunks::<4>().0 {
            rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
        }
    }
    Ok(rgb)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pixels() -> Pixels {
        Pixels {
            width: 128,
            height: 128,
            screen: [128., 128.],
            rgb: Zeroizing::new(vec![0; 128 * 128 * 3]),
        }
    }
    #[test]
    fn large_displays_preserve_input_coordinates_with_bounded_frames() {
        let frame = Pixels {
            width: 2560,
            height: 1600,
            screen: [2560., 1600.],
            rgb: Zeroizing::new(vec![7; 2560 * 1600 * 3]),
        }
        .bounded();
        assert_eq!((frame.width, frame.height), (1920, 1200));
        assert_eq!(frame.screen, [2560., 1600.]);
        assert_eq!(frame.rgb.len(), 1920 * 1200 * 3);
    }
    #[test]
    fn dirty_rectangle_has_a_base_and_idle_sends_nothing() {
        let mut encoder = Encoder::new();
        let first = encoder.encode(pixels(), 0, false).unwrap().unwrap().0;
        assert_eq!(&first[..4], b"NYXD");
        assert_eq!(u64::from_be_bytes(first[16..24].try_into().unwrap()), 0);
        assert!(encoder.encode(pixels(), 1, false).unwrap().is_none());
        let mut changed = pixels();
        changed.rgb[127 * 128 * 3 + 127 * 3] = 255;
        let delta = encoder.encode(changed, 8, false).unwrap().unwrap().0;
        assert_eq!(u64::from_be_bytes(delta[16..24].try_into().unwrap()), 8);
        assert_eq!(u16::from_be_bytes(delta[8..10].try_into().unwrap()), 64);
        assert_eq!(u16::from_be_bytes(delta[12..14].try_into().unwrap()), 64);
        assert!(encoder.encode(pixels(), 9, true).unwrap().is_some());
    }
}
