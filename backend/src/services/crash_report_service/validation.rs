use super::*;
use sha2::{Digest, Sha256};

static VERSION: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"^[0-9A-Za-z.+-]{1,32}$").expect("static pattern")
});
static BUILD: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"^[0-9A-Za-z.+-]{1,64}$").expect("static pattern")
});
static OS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"^[0-9]{1,3}(\.[0-9]{1,3})?$").expect("static pattern")
});
static ASSET: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"^assets/[A-Za-z0-9_.-]{1,96}\.js$").expect("static pattern")
});
static IMAGE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"^[A-Za-z0-9_.+-]{1,64}$").expect("static pattern")
});
fn uuid(value: &str, v4: bool) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| {
        id.hyphenated().to_string() == value.to_ascii_lowercase()
            && (!v4 || id.get_version_num() == 4)
    })
}
/// Bound before parsing; only the normalized UTC value is stored or exported.
pub fn captured_at(value: &str) -> AppResult<DateTime<Utc>> {
    if value.len() > 40 {
        return Err(AppError::ValidationError("Invalid crash report".into()));
    }
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|_| AppError::ValidationError("Invalid crash report".into()))
}
pub fn validate(report: &CrashReportV1, now: DateTime<Utc>) -> AppResult<()> {
    let bad = || AppError::ValidationError("Invalid crash report".into());
    let captured = captured_at(&report.captured_at)?;
    if report.schema_version != 1
        || !uuid(&report.event_id, true)
        || captured > now + chrono::Duration::minutes(5)
        || captured < now - chrono::Duration::hours(24)
        || !VERSION.is_match(&report.app_version)
        || !BUILD.is_match(&report.app_build)
        || !OS.is_match(&report.os_version)
        || report.artifact_id.as_ref().is_some_and(|v| !uuid(v, false))
        || (report.platform == Platform::Web && report.artifact_id.is_none())
        || report.js_frames.len() > 64
        || report.native_frames.len() > 64
        || report.native_images.len() > 16
    {
        return Err(bad());
    }
    let native = report.kind == Kind::NativeCrash;
    if native != report.native_signal.is_some()
        || (native
            && (report.platform != Platform::Ios
                || !report.js_frames.is_empty()
                || report.runtime.is_some()))
        || (!native && (!report.native_images.is_empty() || !report.native_frames.is_empty()))
        || report.runtime.as_ref().is_some_and(|runtime| {
            (report.platform != Platform::Web && runtime.name != RuntimeName::Hermes)
                || (report.platform == Platform::Web && runtime.name == RuntimeName::Hermes)
        })
    {
        return Err(bad());
    }
    for frame in &report.js_frames {
        let valid_asset = if report.platform == Platform::Web {
            ASSET.is_match(&frame.asset)
                || matches!(frame.asset.as_str(), "<external>" | "<inline>")
        } else {
            matches!(
                frame.asset.as_str(),
                "main.jsbundle" | "index.android.bundle" | "<native>" | "<external>"
            )
        };
        if !valid_asset || frame.line > 10_000_000 || frame.column > 10_000_000 {
            return Err(bad());
        }
    }
    for image in &report.native_images {
        if !uuid(&image.uuid, false) || !IMAGE.is_match(&image.name) || image.size > i64::MAX as u64
        {
            return Err(bad());
        }
    }
    for frame in &report.native_frames {
        if frame
            .image
            .is_some_and(|i| usize::from(i) >= report.native_images.len())
            || frame.offset > i64::MAX as u64
        {
            return Err(bad());
        }
        if let Some(index) = frame.image
            && frame.offset >= report.native_images[usize::from(index)].size
        {
            return Err(bad());
        }
    }
    Ok(())
}
pub fn severity(kind: Kind) -> &'static str {
    if matches!(kind, Kind::NativeCrash | Kind::JsFatal) {
        "fatal"
    } else {
        "error"
    }
}
pub fn fingerprint(report: &CrashReportV1) -> String {
    let frames: Vec<_> = report
        .js_frames
        .iter()
        .take(8)
        .map(|f| (&f.asset, f.line, f.column))
        .collect();
    let native: Vec<_> = report
        .native_frames
        .iter()
        .take(8)
        .map(|f| {
            (
                f.image
                    .and_then(|i| report.native_images.get(usize::from(i)))
                    .map(|i| &i.uuid),
                f.offset,
            )
        })
        .collect();
    hex::encode(Sha256::digest(
        serde_json::to_vec(&(
            report.platform,
            report.kind,
            report.exception_class,
            report.native_signal,
            frames,
            native,
        ))
        .expect("closed record"),
    ))
}

pub fn valid_release(version: &str, build: &str, artifact: Option<&str>) -> bool {
    VERSION.is_match(version) && BUILD.is_match(build) && artifact.is_none_or(|id| uuid(id, false))
}
