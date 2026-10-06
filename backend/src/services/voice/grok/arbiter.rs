//! A provider response ending is not the same as its audio having played.
use crate::errors::{AppError, AppResult};
use std::collections::{HashSet, VecDeque};

#[derive(Default)]
pub struct Arbiter {
    pub active: Option<String>,
    pub starting: bool,
    pub playback_end_ms: i64,
    pub playback_ms: i64,
    pub unresolved: HashSet<String>,
    pub cancelled: HashSet<String>,
    lines: VecDeque<String>,
    natural: bool,
}
impl Arbiter {
    pub fn line(&mut self, text: String) -> AppResult<()> {
        if text.len() > 8000 || self.lines.len() >= 32 {
            return Err(AppError::VoiceProviderUnavailable);
        }
        self.lines.push_back(text);
        Ok(())
    }
    pub fn respond(&mut self) {
        self.natural = true;
    }
    pub fn take_next(&mut self) -> Option<Option<String>> {
        if self.starting
            || self.active.is_some()
            || !self.unresolved.is_empty()
            || self.playback_ms < self.playback_end_ms
        {
            return None;
        }
        let next = if let Some(line) = self.lines.pop_front() {
            Some(Some(line))
        } else if std::mem::take(&mut self.natural) {
            Some(None)
        } else {
            None
        };
        if next.is_some() {
            self.starting = true;
        }
        next
    }
    pub fn created(&mut self, id: String) -> AppResult<()> {
        if !self.starting || self.active.is_some() {
            return Err(AppError::VoiceProviderUnavailable);
        }
        self.starting = false;
        self.active = Some(id);
        Ok(())
    }
    pub fn done(&mut self, id: &str) -> AppResult<()> {
        if self.active.as_deref() != Some(id) {
            return Err(AppError::VoiceProviderUnavailable);
        }
        self.active = None;
        Ok(())
    }
}
