//! A fresh server clock snapshot; no user-supplied instant can replace it.
use crate::errors::{AppError, AppResult};
use chrono::Utc;
use serde_json::{Value, json};

pub(crate) fn read(args: &Value) -> AppResult<Value> {
    let timezone: chrono_tz::Tz = args["timezone"]
        .as_str()
        .unwrap_or("UTC")
        .parse()
        .map_err(|_| AppError::ValidationError("timezone must be an IANA timezone".into()))?;
    let now = Utc::now();
    Ok(
        json!({"utc":now.to_rfc3339(),"local":now.with_timezone(&timezone).to_rfc3339(),
        "timezone":timezone.name(),"unix_ms":now.timestamp_millis()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_is_fresh_and_uses_the_requested_timezone() {
        let before = Utc::now().timestamp_millis();
        let result = read(&json!({"timezone":"Asia/Singapore"})).unwrap();
        let after = Utc::now().timestamp_millis();
        let actual = result["unix_ms"].as_i64().unwrap();
        assert!((before..=after).contains(&actual));
        let utc = chrono::DateTime::parse_from_rfc3339(result["utc"].as_str().unwrap()).unwrap();
        let local =
            chrono::DateTime::parse_from_rfc3339(result["local"].as_str().unwrap()).unwrap();
        assert_eq!(utc, local);
        assert_eq!(local.offset().local_minus_utc(), 8 * 3600);
        assert!(read(&json!({"timezone":"a made up timezone"})).is_err());
    }
}
