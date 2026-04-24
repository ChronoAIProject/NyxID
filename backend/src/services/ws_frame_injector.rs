use sha2::{Digest, Sha256};

use crate::errors::{AppError, AppResult};
use crate::models::ws_frame_injection::{
    WsFrameDirection, WsFrameInjection, WsFrameKind, WsFrameTrigger,
};

pub const MAX_RULES_PER_SERVICE: usize = 4;
pub const MAX_TEMPLATE_BYTES: usize = 4096;
pub const MAX_INJECTIONS_PER_CONNECTION: usize = 8;

#[derive(Debug, Default, Clone)]
pub struct InjectorState {
    pub downstream_frame_index: usize,
    pub upstream_frame_index: usize,
    pub total_injections_fired: usize,
}

#[derive(Clone, Debug)]
pub struct IncomingFrame {
    pub direction: WsFrameDirection,
    pub kind: WsFrameKind,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WsFrame {
    pub kind: WsFrameKind,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InjectionAction {
    pub send_frame: WsFrame,
    pub forward_original: bool,
    pub trigger_kind: &'static str,
    pub frame_index_in: usize,
    pub direction: WsFrameDirection,
    pub credential_sha256_prefix: String,
}

pub fn evaluate(
    rules: &[WsFrameInjection],
    state: &mut InjectorState,
    frame: &IncomingFrame,
    credential: &str,
) -> Option<InjectionAction> {
    let frame_index_in = match frame.direction {
        WsFrameDirection::Downstream => {
            let idx = state.downstream_frame_index;
            state.downstream_frame_index = state.downstream_frame_index.saturating_add(1);
            idx
        }
        WsFrameDirection::Upstream => {
            let idx = state.upstream_frame_index;
            state.upstream_frame_index = state.upstream_frame_index.saturating_add(1);
            idx
        }
    };

    if state.total_injections_fired >= MAX_INJECTIONS_PER_CONNECTION {
        return None;
    }

    for rule in rules {
        if rule.direction != frame.direction || rule.frame_kind != frame.kind {
            continue;
        }

        let Some(trigger_kind) = trigger_matches(&rule.trigger, frame, frame_index_in) else {
            continue;
        };

        state.total_injections_fired = state.total_injections_fired.saturating_add(1);
        let payload = rule
            .template
            .replace("${credential}", credential)
            .into_bytes();
        return Some(InjectionAction {
            send_frame: WsFrame {
                kind: rule.frame_kind,
                payload,
            },
            forward_original: !rule.consume_trigger,
            trigger_kind,
            frame_index_in,
            direction: rule.direction,
            credential_sha256_prefix: credential_hash_prefix(credential),
        });
    }

    None
}

pub fn validate_rules(rules: &[WsFrameInjection]) -> AppResult<()> {
    if rules.len() > MAX_RULES_PER_SERVICE {
        return Err(AppError::ValidationError(format!(
            "ws_frame_injections must not exceed {MAX_RULES_PER_SERVICE} entries"
        )));
    }

    for (idx, rule) in rules.iter().enumerate() {
        if rule.template.len() > MAX_TEMPLATE_BYTES {
            return Err(AppError::ValidationError(format!(
                "ws_frame_injections[{idx}].template must not exceed {MAX_TEMPLATE_BYTES} bytes"
            )));
        }
        validate_template_does_not_embed_credentials(idx, &rule.template)?;
        if let WsFrameTrigger::JsonFieldEquals { path, .. } = &rule.trigger {
            validate_json_path(idx, path)?;
        }
    }

    Ok(())
}

pub fn credential_hash_prefix(credential: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(credential.as_bytes());
    hex::encode(hasher.finalize())
        .chars()
        .take(12)
        .collect::<String>()
}

fn trigger_matches(
    trigger: &WsFrameTrigger,
    frame: &IncomingFrame,
    frame_index_in: usize,
) -> Option<&'static str> {
    match trigger {
        WsFrameTrigger::FirstFrameFromDownstream => {
            (frame.direction == WsFrameDirection::Downstream && frame_index_in == 0)
                .then_some("first_frame_from_downstream")
        }
        WsFrameTrigger::FrameIndexFromDownstream { index } => {
            (frame.direction == WsFrameDirection::Downstream && frame_index_in == *index)
                .then_some("frame_index_from_downstream")
        }
        WsFrameTrigger::JsonFieldEquals { path, value } => {
            if frame.kind != WsFrameKind::Text {
                return None;
            }
            let text = std::str::from_utf8(&frame.payload).ok()?;
            let json: serde_json::Value = serde_json::from_str(text).ok()?;
            let actual = get_json_path_value(&json, path)?;
            (actual == value).then_some("json_field_equals")
        }
    }
}

fn get_json_path_value<'a>(
    value: &'a serde_json::Value,
    path: &str,
) -> Option<&'a serde_json::Value> {
    let path = path.strip_prefix("$.")?;
    let mut current = value;
    for part in path.split('.') {
        if part.is_empty() {
            return None;
        }
        current = current.get(part)?;
    }
    Some(current)
}

fn validate_json_path(idx: usize, path: &str) -> AppResult<()> {
    if path.len() > 256 || !path.starts_with("$.") {
        return Err(AppError::ValidationError(format!(
            "ws_frame_injections[{idx}].trigger.path must use simple dot notation like $.type"
        )));
    }

    for part in path.trim_start_matches("$.").split('.') {
        if part.is_empty()
            || !part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(AppError::ValidationError(format!(
                "ws_frame_injections[{idx}].trigger.path must use simple dot notation like $.type"
            )));
        }
    }

    Ok(())
}

fn validate_template_does_not_embed_credentials(idx: usize, template: &str) -> AppResult<()> {
    let stripped = template.replace("${credential}", "");
    if stripped.contains("nyxid_") || contains_jwt_like_literal(&stripped) {
        return Err(AppError::ValidationError(format!(
            "ws_frame_injections[{idx}].template must not embed credential-looking literals"
        )));
    }
    Ok(())
}

fn contains_jwt_like_literal(value: &str) -> bool {
    value
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.'))
        .any(|segment| segment.starts_with("eyJ"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ha_rule() -> WsFrameInjection {
        WsFrameInjection {
            trigger: WsFrameTrigger::JsonFieldEquals {
                path: "$.type".to_string(),
                value: serde_json::json!("auth_required"),
            },
            template: r#"{"type":"auth","access_token":"${credential}"}"#.to_string(),
            frame_kind: WsFrameKind::Text,
            consume_trigger: true,
            direction: WsFrameDirection::Downstream,
        }
    }

    fn text_downstream(payload: &str) -> IncomingFrame {
        IncomingFrame {
            direction: WsFrameDirection::Downstream,
            kind: WsFrameKind::Text,
            payload: payload.as_bytes().to_vec(),
        }
    }

    #[test]
    fn first_frame_trigger_fires_exactly_once() {
        let rules = vec![WsFrameInjection {
            trigger: WsFrameTrigger::FirstFrameFromDownstream,
            template: "hello ${credential}".to_string(),
            frame_kind: WsFrameKind::Text,
            consume_trigger: true,
            direction: WsFrameDirection::Downstream,
        }];
        let mut state = InjectorState::default();

        assert!(evaluate(&rules, &mut state, &text_downstream("first"), "CRED").is_some());
        assert!(evaluate(&rules, &mut state, &text_downstream("second"), "CRED").is_none());
    }

    #[test]
    fn json_path_match_on_nested_field() {
        let rules = vec![WsFrameInjection {
            trigger: WsFrameTrigger::JsonFieldEquals {
                path: "$.hello.type".to_string(),
                value: serde_json::json!("auth_required"),
            },
            template: "auth ${credential}".to_string(),
            frame_kind: WsFrameKind::Text,
            consume_trigger: true,
            direction: WsFrameDirection::Downstream,
        }];
        let mut state = InjectorState::default();

        let action = evaluate(
            &rules,
            &mut state,
            &text_downstream(r#"{"hello":{"type":"auth_required"}}"#),
            "TOKEN",
        )
        .expect("match");
        assert_eq!(action.trigger_kind, "json_field_equals");
    }

    #[test]
    fn json_path_no_match_does_nothing() {
        let rules = vec![ha_rule()];
        let mut state = InjectorState::default();

        assert!(
            evaluate(
                &rules,
                &mut state,
                &text_downstream(r#"{"type":"auth_ok"}"#),
                "TOKEN",
            )
            .is_none()
        );
    }

    #[test]
    fn binary_frame_trigger_ignored_when_frame_kind_mismatch() {
        let rules = vec![ha_rule()];
        let mut state = InjectorState::default();
        let frame = IncomingFrame {
            direction: WsFrameDirection::Downstream,
            kind: WsFrameKind::Binary,
            payload: br#"{"type":"auth_required"}"#.to_vec(),
        };

        assert!(evaluate(&rules, &mut state, &frame, "TOKEN").is_none());
    }

    #[test]
    fn template_substitution_correct() {
        let rules = vec![ha_rule()];
        let mut state = InjectorState::default();

        let action = evaluate(
            &rules,
            &mut state,
            &text_downstream(r#"{"type":"auth_required"}"#),
            "TEST_CRED",
        )
        .expect("match");
        assert_eq!(
            String::from_utf8(action.send_frame.payload).expect("utf8"),
            r#"{"type":"auth","access_token":"TEST_CRED"}"#
        );
    }

    #[test]
    fn injection_cap_honored() {
        let rules = vec![WsFrameInjection {
            trigger: WsFrameTrigger::JsonFieldEquals {
                path: "$.type".to_string(),
                value: serde_json::json!("auth_required"),
            },
            template: "auth ${credential}".to_string(),
            frame_kind: WsFrameKind::Text,
            consume_trigger: true,
            direction: WsFrameDirection::Downstream,
        }];
        let mut state = InjectorState::default();

        for _ in 0..MAX_INJECTIONS_PER_CONNECTION {
            assert!(
                evaluate(
                    &rules,
                    &mut state,
                    &text_downstream(r#"{"type":"auth_required"}"#),
                    "TOKEN",
                )
                .is_some()
            );
        }
        assert!(
            evaluate(
                &rules,
                &mut state,
                &text_downstream(r#"{"type":"auth_required"}"#),
                "TOKEN",
            )
            .is_none()
        );
    }

    #[test]
    fn consume_trigger_false_leaves_frame_visible() {
        let mut rule = ha_rule();
        rule.consume_trigger = false;
        let mut state = InjectorState::default();

        let action = evaluate(
            &[rule],
            &mut state,
            &text_downstream(r#"{"type":"auth_required"}"#),
            "TOKEN",
        )
        .expect("match");
        assert!(action.forward_original);
    }

    #[test]
    fn malformed_json_does_not_panic_or_fire() {
        let rules = vec![ha_rule()];
        let mut state = InjectorState::default();

        assert!(evaluate(&rules, &mut state, &text_downstream("{"), "TOKEN").is_none());
    }

    #[test]
    fn validation_rejects_embedded_nyxid_token_literal() {
        let mut rule = ha_rule();
        rule.template = r#"{"token":"nyxid_ag_bad"}"#.to_string();
        assert!(validate_rules(&[rule]).is_err());
    }
}
