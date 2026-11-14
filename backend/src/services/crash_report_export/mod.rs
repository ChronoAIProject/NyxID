//! Optional vendor transport. Only the owner-free sanitized projection can cross this boundary.
pub mod sentry;
pub mod worker;
use crate::models::client_error_report::{CrashReportV1, Platform};
use std::{sync::Arc, time::Duration};

pub struct SanitizedReport<'a> {
    pub report: &'a CrashReportV1,
    pub environment: &'a str,
    pub severity: &'a str,
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
    pub fn from_config(config: &crate::config::AppConfig) -> Result<Self, &'static str> {
        let value = &config.crash_reporting;
        let destination =
            |v: &Option<crate::config::CrashDestinationConfig>| -> Result<
                Option<Arc<dyn CrashReportExporter>>,
                &'static str,
            > {
            v.as_ref()
                .map(|v| {
                    sentry::SentryExporter::new(v)
                        .map(|e| Arc::new(e) as Arc<dyn CrashReportExporter>)
                })
                .transpose()
            };
        Ok(Self {
            web: destination(&value.web)?,
            mobile: destination(&value.mobile)?,
            retention_days: value.retention_days,
            interval: Duration::from_secs(value.export_interval_secs),
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
