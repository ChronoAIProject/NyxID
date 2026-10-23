//! Purpose-separated Telegram start payloads reuse the existing atomic proofs.
use zeroize::Zeroizing;

pub const PERSONAL_PREFIX: &str = "nyxpersonal_";

pub fn onboarding_code(code: &str) -> bool {
    code.len() == 34 && code.starts_with("O-") && code[2..].bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn onboarding_command(text: &str, username: &str) -> Option<Zeroizing<String>> {
    let mut words = text.split_whitespace();
    let verb = words.next()?;
    let (command, target) = verb
        .split_once('@')
        .map_or((verb, None), |(c, t)| (c, Some(t)));
    if command != "/start" || target.is_some_and(|t| !t.eq_ignore_ascii_case(username)) {
        return None;
    }
    let code = words.next()?.strip_prefix("nyxonboard_")?;
    if words.next().is_some() || !onboarding_code(code) {
        return None;
    }
    Some(Zeroizing::new(format!("/confirm {code}")))
}

pub fn personal_command(text: &str, username: &str) -> Option<Zeroizing<String>> {
    let mut words = text.split_whitespace();
    let verb = words.next()?;
    let (command, target) = verb
        .split_once('@')
        .map_or((verb, None), |(c, t)| (c, Some(t)));
    if command != "/start" || target.is_some_and(|t| !t.eq_ignore_ascii_case(username)) {
        return None;
    }
    let code = words.next()?.strip_prefix(PERSONAL_PREFIX)?;
    if words.next().is_some()
        || code.len() != 11
        || !code.is_ascii()
        || !code.starts_with("P-")
        || code.as_bytes()[6] != b'-'
        || !code[2..6]
            .bytes()
            .chain(code[7..].bytes())
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    {
        return None;
    }
    Some(Zeroizing::new(format!("/confirm {code}")))
}

/// This predicate is evaluated against authenticated provider input before
/// private setup messages are scrubbed. Unknown or quoted provenance fails closed.
pub fn original_message(update: &serde_json::Value) -> bool {
    super::telegram_input::original(update)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn personal_payload_is_purpose_bound_and_has_no_command_injection() {
        assert_eq!(
            personal_command("/start@manager nyxpersonal_P-ABCD-1234", "Manager")
                .unwrap()
                .as_str(),
            "/confirm P-ABCD-1234"
        );
        for text in [
            "/start@other nyxpersonal_P-ABCD-1234",
            "/start nyxpersonal_P-ABCD-1234 extra",
            "/start nyxonboard_P-ABCD-1234",
            "/start nyxpersonal_P-ABCD-12é",
            "/start P-ABCD-1234",
        ] {
            assert!(personal_command(text, "manager").is_none());
        }
    }
    #[test]
    fn quoted_forwarded_edited_and_unknown_input_cannot_be_pairing_proof() {
        let original = serde_json::json!({"message":{"message_id":1,"from":{"is_bot":false}}});
        assert!(original_message(&original));
        for field in [
            "quote",
            "external_reply",
            "forward_origin",
            "forward_date",
            "via_bot",
            "sender_chat",
        ] {
            let mut update = original.clone();
            update["message"][field] = serde_json::json!({});
            assert!(!original_message(&update));
        }
        assert!(!original_message(
            &serde_json::json!({"edited_message":original["message"]})
        ));
        assert!(!original_message(
            &serde_json::json!({"message":{"text":"yes"}})
        ));
    }
}
