use chrono::{DateTime, Duration, Utc};

#[cfg(test)]
use crate::crypto::device_code::generate_user_code;
use crate::errors::{AppError, AppResult};
use crate::models::device_code::{DeviceCode, UserCodeGen};

use super::DEVICE_CODE_ROTATE_SECS;

pub(super) fn rotate_user_code_if_needed_with_generator<F>(
    row: &mut DeviceCode,
    now: DateTime<Utc>,
    mut user_code_generator: F,
) -> AppResult<String>
where
    F: FnMut() -> String,
{
    if now.signed_duration_since(row.last_rotated_at) > Duration::seconds(DEVICE_CODE_ROTATE_SECS) {
        let new_code = user_code_generator();
        row.user_code_history.insert(
            0,
            UserCodeGen {
                code: new_code.clone(),
                generated_at: now,
            },
        );
        row.user_code_history.truncate(4);
        row.last_rotated_at = now;
        return Ok(new_code);
    }

    row.user_code_history
        .first()
        .map(|generation| generation.code.clone())
        .ok_or_else(|| AppError::Internal("device code has no user code history".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::device_code::DeviceCodeStatus;
    use uuid::Uuid;

    fn row_with_history(codes: &[&str], last_rotated_at: DateTime<Utc>) -> DeviceCode {
        DeviceCode {
            id: Uuid::new_v4().to_string(),
            device_code_hash: "deadbeef".repeat(8),
            device_pubkey: vec![1u8; 32],
            hw_id: "esp32-cam".to_string(),
            suggested_label: None,
            user_code_history: codes
                .iter()
                .map(|code| UserCodeGen {
                    code: (*code).to_string(),
                    generated_at: last_rotated_at,
                })
                .collect(),
            status: DeviceCodeStatus::Pending,
            approved_by_user_id: None,
            approved_org_id: None,
            issued_api_key_id: None,
            issued_node_id: None,
            delivery_api_key_encrypted: None,
            delivery_refresh_token_encrypted: None,
            refresh_token_hash: None,
            failed_poll_count: 0,
            locked_until: None,
            lock_alert_sent_at: None,
            expires_at: Utc::now() + Duration::minutes(15),
            created_at: Utc::now(),
            last_polled_at: None,
            last_poll_timestamp: None,
            last_rotated_at,
        }
    }

    #[test]
    fn no_rotation_returns_current_user_code() {
        let now = Utc::now();
        let mut row = row_with_history(&["AAAA-BBBB-CCCC"], now);

        let current =
            rotate_user_code_if_needed_with_generator(&mut row, now, generate_user_code).unwrap();

        assert_eq!(current, "AAAA-BBBB-CCCC");
        assert_eq!(row.user_code_history.len(), 1);
        assert_eq!(row.last_rotated_at, now);
    }

    #[test]
    fn rotation_prepends_new_code_and_retains_four_generations() {
        let now = Utc::now();
        let old_rotated_at = now - Duration::seconds(DEVICE_CODE_ROTATE_SECS + 1);
        let mut row = row_with_history(
            &[
                "AAAA-BBBB-CCCC",
                "DDDD-EEEE-FFFF",
                "GGGG-HHHH-JJJJ",
                "KKKK-LLLL-MMMM",
            ],
            old_rotated_at,
        );

        let current =
            rotate_user_code_if_needed_with_generator(&mut row, now, generate_user_code).unwrap();

        assert_eq!(row.user_code_history.len(), 4);
        assert_eq!(row.user_code_history[0].code, current);
        assert_eq!(row.user_code_history[0].generated_at, now);
        assert_eq!(row.user_code_history[1].code, "AAAA-BBBB-CCCC");
        assert_eq!(row.user_code_history[2].code, "DDDD-EEEE-FFFF");
        assert_eq!(row.user_code_history[3].code, "GGGG-HHHH-JJJJ");
        assert_eq!(row.last_rotated_at, now);
    }
}
