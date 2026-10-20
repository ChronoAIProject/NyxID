use super::{Pixels, rgb_from_bgra};
use anyhow::{Context, Result};
use screencapturekit::{cv::CVPixelBufferLockFlags, prelude::*};
use std::sync::{Arc, Mutex};

pub struct Capture {
    stream: SCStream,
    latest: Arc<Mutex<Option<Pixels>>>,
}
impl Capture {
    pub fn for_display(_display: nyxid_machine::desktop::Display) -> Result<Self> {
        Self::new()
    }
    pub fn new() -> Result<Self> {
        let content = SCShareableContent::get()?;
        let display = content
            .displays()
            .into_iter()
            .next()
            .context("Screen Recording permission or display unavailable")?;
        let screen = [f64::from(display.width()), f64::from(display.height())];
        let scale = (1920.0 / screen[0]).min(1200.0 / screen[1]).min(1.0);
        let width = (screen[0] * scale) as u32;
        let height = (screen[1] * scale) as u32;
        let filter = SCContentFilter::create()
            .with_display(&display)
            .with_excluding_windows(&[])
            .build()?;
        let config = SCStreamConfiguration::new()
            .with_width(width)
            .with_height(height)
            .with_pixel_format(PixelFormat::BGRA)
            .with_shows_cursor(true)
            .with_minimum_frame_interval(&CMTime::new(1, 30));
        let mut stream = SCStream::new(&filter, &config)?;
        let latest = Arc::new(Mutex::new(None));
        let received = latest.clone();
        stream.add_output_handler(
            move |sample: CMSampleBuffer, kind: SCStreamOutputType| {
                if kind != SCStreamOutputType::Screen {
                    return;
                }
                let Some(buffer) = sample.pixel_buffer() else {
                    return;
                };
                let Ok(guard) = buffer.lock(CVPixelBufferLockFlags::READ_ONLY) else {
                    return;
                };
                // The read lock owns the pointer for the entire copy.
                let Some(bytes) = (unsafe { guard.as_slice() }) else {
                    return;
                };
                let width = buffer.width() as u32;
                let height = buffer.height() as u32;
                if let Ok(rgb) = rgb_from_bgra(width, height, buffer.bytes_per_row(), bytes)
                    && let Ok(mut latest) = received.lock()
                {
                    *latest = Some(Pixels {
                        width,
                        height,
                        screen,
                        rgb,
                    });
                }
            },
            SCStreamOutputType::Screen,
        )?;
        stream.start_capture()?;
        Ok(Self { stream, latest })
    }
    pub fn capture(&mut self) -> Result<Option<Pixels>> {
        Ok(self
            .latest
            .lock()
            .map_err(|_| anyhow::anyhow!("desktop capture stopped"))?
            .as_ref()
            .cloned())
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.stream.stop_capture();
    }
}
