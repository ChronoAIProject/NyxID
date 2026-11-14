//! Optional vendor transport. Only the owner-free sanitized projection can cross this boundary.
pub mod sentry;
pub mod worker;
use crate::models::client_error_report::{CrashReportV1, Platform};
use std::{sync::Arc, time::Duration};

pub struct SanitizedReport<'a> {
    pub report: &'a CrashReportV1,
    pub environment: &'a str,
    pub severity: &'a str,
    pub fingerprint: &'a str,
}
#[derive(Debug, PartialEq, Eq)]
pub enum SubmitOutcome {
    Exported,
    Retry { after: u64, code: &'static str },
    Unknown,
    Failed,
}
#[async_trait::async_trait]
pub trait CrashReportExporter: Send + Sync {
    fn destination_id(&self) -> &str;
    async fn submit(&self, report: &SanitizedReport<'_>) -> SubmitOutcome;
}
#[derive(Clone)]
pub struct CrashConfig {
    pub web: Option<Arc<dyn CrashReportExporter>>,
    pub mobile: Option<Arc<dyn CrashReportExporter>>,
    pub retention_days: u32,
    pub interval: Duration,
}
impl Default for CrashConfig {
    fn default() -> Self {
        Self {
            web: None,
            mobile: None,
            retention_days: 30,
            interval: Duration::from_secs(10),
        }
    }
}
impl CrashConfig {
    pub fn from_env() -> Result<Self, &'static str> {
        let destination = |key| -> Result<Option<Arc<dyn CrashReportExporter>>, &'static str> {
            match std::env::var(key) {
                Ok(value) if !value.is_empty() => {
                    Ok(Some(Arc::new(sentry::SentryExporter::new(&value)?)))
                }
                Ok(_) | Err(std::env::VarError::NotPresent) => Ok(None),
                Err(_) => Err("Invalid crash destination configuration"),
            }
        };
        let retention_days = std::env::var("CRASH_REPORTS_RETENTION_DAYS")
            .unwrap_or_else(|_| "30".into())
            .parse::<u32>()
            .map_err(|_| "Invalid crash retention")?;
        if !(1..=30).contains(&retention_days) {
            return Err("Crash retention must be 1–30 days");
        }
        let interval = std::env::var("CRASH_REPORTS_EXPORT_INTERVAL_SECS")
            .unwrap_or_else(|_| "10".into())
            .parse::<u64>()
            .map_err(|_| "Invalid crash export interval")?;
        if interval > 86400 {
            return Err("Crash export interval exceeds one day");
        }
        Ok(Self {
            web: destination("CRASH_REPORTS_SENTRY_DSN_WEB")?,
            mobile: destination("CRASH_REPORTS_SENTRY_DSN_MOBILE")?,
            retention_days,
            interval: Duration::from_secs(interval),
        })
    }
    pub fn destination(&self, platform: Platform) -> Option<&dyn CrashReportExporter> {
        if platform == Platform::Web {
            self.web.as_deref()
        } else {
            self.mobile.as_deref()
        }
    }
    pub fn destination_ids(&self) -> Vec<String> {
        let mut ids: Vec<_> = self
            .web
            .iter()
            .chain(self.mobile.iter())
            .map(|d| d.destination_id().to_owned())
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }
}
