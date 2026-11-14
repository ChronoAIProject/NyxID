//! Closed diagnostics records. Strings are validated before storage; never log a record.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "client_error_reports";
pub const COUNTERS: &str = "client_error_report_counters";

macro_rules! closed_enum {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
        pub enum $name { $(#[serde(rename = $value)] $variant),+ }
        impl $name {
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $value),+ }
            }
        }
    };
}
closed_enum!(Platform { Web => "web", Ios => "ios", Android => "android" });
closed_enum!(Kind {
    NativeCrash => "native_crash",
    JsFatal => "js_fatal",
    JsError => "js_error",
    UnhandledRejection => "unhandled_rejection",
    RenderError => "render_error",
    RecoverableError => "recoverable_error",
    ResourceError => "resource_error"
});
closed_enum!(ExceptionClass {
    Error => "Error",
    TypeError => "TypeError",
    ReferenceError => "ReferenceError",
    RangeError => "RangeError",
    SyntaxError => "SyntaxError",
    EvalError => "EvalError",
    UriError => "URIError",
    AggregateError => "AggregateError",
    ChunkLoadError => "ChunkLoadError",
    NonError => "NonError",
    Other => "Other",
    Unknown => "Unknown"
});
closed_enum!(NativeSignal {
    Segv => "SIGSEGV",
    Bus => "SIGBUS",
    Abrt => "SIGABRT",
    Ill => "SIGILL",
    Fpe => "SIGFPE",
    Trap => "SIGTRAP",
    Sys => "SIGSYS",
    Other => "Other"
});
closed_enum!(RuntimeName {
    Chrome => "chrome",
    Safari => "safari",
    Firefox => "firefox",
    Edge => "edge",
    Hermes => "hermes",
    Other => "other"
});
closed_enum!(ExportStatus {
    Disabled => "disabled",
    Pending => "pending",
    Dispatching => "dispatching",
    Exported => "exported",
    Unknown => "unknown",
    Failed => "failed",
    Discarded => "discarded"
});

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Runtime {
    pub name: RuntimeName,
    pub major: u16,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JsFrame {
    pub asset: String,
    pub line: u32,
    pub column: u32,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeImage {
    pub uuid: String,
    pub name: String,
    pub size: u64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeFrame {
    pub image: Option<u8>,
    pub offset: u64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrashReportV1 {
    pub schema_version: u8,
    pub event_id: String,
    // Wire timestamp is RFC3339, validated before admission. Database timestamps below use BSON.
    pub captured_at: String,
    pub platform: Platform,
    pub kind: Kind,
    pub exception_class: ExceptionClass,
    #[serde(default)]
    pub native_signal: Option<NativeSignal>,
    pub app_version: String,
    pub app_build: String,
    #[serde(default)]
    pub artifact_id: Option<String>,
    pub os_version: String,
    #[serde(default)]
    pub runtime: Option<Runtime>,
    #[serde(default)]
    pub js_frames: Vec<JsFrame>,
    #[serde(default)]
    pub native_images: Vec<NativeImage>,
    #[serde(default)]
    pub native_frames: Vec<NativeFrame>,
}
impl std::fmt::Debug for CrashReportV1 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CrashReportV1([redacted])")
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Export {
    pub status: ExportStatus,
    #[serde(default)]
    pub destination_id: Option<String>,
    #[serde(default)]
    pub attempts: u32,
    #[serde(default)]
    pub lease_id: Option<String>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub lease_until: Option<DateTime<Utc>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub next_attempt_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub deadline: DateTime<Utc>,
    #[serde(default)]
    pub last_error_code: Option<String>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub exported_at: Option<DateTime<Utc>>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct ClientErrorReport {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub event_id: String,
    pub consent_revision: String,
    pub report: CrashReportV1,
    pub environment: String,
    pub severity: String,
    pub fingerprint: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub received_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    pub export: Export,
}
impl std::fmt::Debug for ClientErrorReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClientErrorReport([redacted])")
    }
}
