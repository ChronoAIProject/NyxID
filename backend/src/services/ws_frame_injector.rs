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

    // --- credential_hash_prefix ---

    #[test]
    fn credential_hash_prefix_is_12_hex_chars() {
        let prefix = credential_hash_prefix("my-secret-credential");
        assert_eq!(prefix.len(), 12);
        assert!(prefix.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn credential_hash_prefix_deterministic() {
        assert_eq!(
            credential_hash_prefix("same-input"),
            credential_hash_prefix("same-input"),
        );
    }

    #[test]
    fn credential_hash_prefix_differs_for_different_inputs() {
        assert_ne!(
            credential_hash_prefix("credential-a"),
            credential_hash_prefix("credential-b"),
        );
    }

    // --- validate_rules edge cases ---

    #[test]
    fn validation_accepts_valid_rules() {
        let rules = vec![ha_rule()];
        assert!(validate_rules(&rules).is_ok());
    }

    #[test]
    fn validation_accepts_empty_rules() {
        assert!(validate_rules(&[]).is_ok());
    }

    #[test]
    fn validation_rejects_too_many_rules() {
        let rules: Vec<WsFrameInjection> =
            (0..MAX_RULES_PER_SERVICE + 1).map(|_| ha_rule()).collect();
        let err = validate_rules(&rules).unwrap_err();
        match err {
            AppError::ValidationError(msg) => {
                assert!(msg.contains("must not exceed"));
            }
            _ => panic!("expected ValidationError, got {:?}", err),
        }
    }

    #[test]
    fn validation_accepts_exactly_max_rules() {
        let rules: Vec<WsFrameInjection> = (0..MAX_RULES_PER_SERVICE).map(|_| ha_rule()).collect();
        assert!(validate_rules(&rules).is_ok());
    }

    #[test]
    fn validation_rejects_oversized_template() {
        let mut rule = ha_rule();
        rule.template = "x".repeat(MAX_TEMPLATE_BYTES + 1);
        let err = validate_rules(&[rule]).unwrap_err();
        match err {
            AppError::ValidationError(msg) => {
                assert!(msg.contains("template must not exceed"));
            }
            _ => panic!("expected ValidationError, got {:?}", err),
        }
    }

    #[test]
    fn validation_accepts_template_at_max_size() {
        let mut rule = ha_rule();
        rule.template = "x".repeat(MAX_TEMPLATE_BYTES);
        assert!(validate_rules(&[rule]).is_ok());
    }

    #[test]
    fn validation_rejects_jwt_like_literal_eyj() {
        let mut rule = ha_rule();
        rule.template =
            r#"{"token":"eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.signature"}"#.to_string();
        assert!(validate_rules(&[rule]).is_err());
    }

    #[test]
    fn validation_allows_credential_placeholder_that_would_expand_to_nyxid() {
        // The placeholder ${credential} should not trigger the check because
        // the check runs on the template with ${credential} stripped out.
        let mut rule = ha_rule();
        rule.template = r#"{"token":"${credential}"}"#.to_string();
        assert!(validate_rules(&[rule]).is_ok());
    }

    // --- validate_json_path ---

    #[test]
    fn json_path_valid_simple() {
        assert!(validate_json_path(0, "$.type").is_ok());
    }

    #[test]
    fn json_path_valid_nested() {
        assert!(validate_json_path(0, "$.hello.world").is_ok());
    }

    #[test]
    fn json_path_valid_underscore_and_hyphen() {
        assert!(validate_json_path(0, "$.my_field.sub-field").is_ok());
    }

    #[test]
    fn json_path_rejects_missing_dollar_dot() {
        assert!(validate_json_path(0, "type").is_err());
        assert!(validate_json_path(0, "$type").is_err());
        assert!(validate_json_path(0, ".type").is_err());
    }

    #[test]
    fn json_path_rejects_too_long() {
        let long_path = format!("$.{}", "a".repeat(256));
        assert!(validate_json_path(0, &long_path).is_err());
    }

    #[test]
    fn json_path_rejects_empty_segment() {
        assert!(validate_json_path(0, "$.hello..world").is_err());
    }

    #[test]
    fn json_path_rejects_special_chars() {
        assert!(validate_json_path(0, "$.hello[0]").is_err());
        assert!(validate_json_path(0, "$.hello/world").is_err());
        assert!(validate_json_path(0, "$.hello world").is_err());
    }

    // --- get_json_path_value ---

    #[test]
    fn get_json_path_value_simple() {
        let json: serde_json::Value = serde_json::json!({"type": "auth"});
        let val = get_json_path_value(&json, "$.type");
        assert_eq!(val.unwrap(), &serde_json::json!("auth"));
    }

    #[test]
    fn get_json_path_value_nested() {
        let json: serde_json::Value = serde_json::json!({"a": {"b": {"c": 42}}});
        let val = get_json_path_value(&json, "$.a.b.c");
        assert_eq!(val.unwrap(), &serde_json::json!(42));
    }

    #[test]
    fn get_json_path_value_returns_none_for_missing_field() {
        let json: serde_json::Value = serde_json::json!({"type": "auth"});
        assert!(get_json_path_value(&json, "$.missing").is_none());
    }

    #[test]
    fn get_json_path_value_returns_none_for_non_object_traversal() {
        let json: serde_json::Value = serde_json::json!({"type": "auth"});
        assert!(get_json_path_value(&json, "$.type.nested").is_none());
    }

    #[test]
    fn get_json_path_value_requires_dollar_dot_prefix() {
        let json: serde_json::Value = serde_json::json!({"type": "auth"});
        assert!(get_json_path_value(&json, "type").is_none());
        assert!(get_json_path_value(&json, "$type").is_none());
    }

    #[test]
    fn get_json_path_value_returns_none_on_empty_segment() {
        let json: serde_json::Value = serde_json::json!({"type": "auth"});
        assert!(get_json_path_value(&json, "$.").is_none());
    }

    // --- upstream direction handling ---

    #[test]
    fn upstream_frame_index_tracked_separately() {
        let rules = vec![WsFrameInjection {
            trigger: WsFrameTrigger::FirstFrameFromDownstream,
            template: "auth ${credential}".to_string(),
            frame_kind: WsFrameKind::Text,
            consume_trigger: true,
            direction: WsFrameDirection::Downstream,
        }];
        let mut state = InjectorState::default();

        // Send upstream frames — they should NOT trigger a downstream rule
        let upstream = IncomingFrame {
            direction: WsFrameDirection::Upstream,
            kind: WsFrameKind::Text,
            payload: b"upstream data".to_vec(),
        };
        assert!(evaluate(&rules, &mut state, &upstream, "CRED").is_none());
        assert_eq!(state.upstream_frame_index, 1);
        assert_eq!(state.downstream_frame_index, 0);

        // The first downstream frame should still trigger (index 0)
        assert!(evaluate(&rules, &mut state, &text_downstream("first"), "CRED").is_some());
        assert_eq!(state.downstream_frame_index, 1);
    }

    // --- frame_index trigger ---

    #[test]
    fn frame_index_trigger_fires_on_exact_index() {
        let rules = vec![WsFrameInjection {
            trigger: WsFrameTrigger::FrameIndexFromDownstream { index: 2 },
            template: "auth ${credential}".to_string(),
            frame_kind: WsFrameKind::Text,
            consume_trigger: false,
            direction: WsFrameDirection::Downstream,
        }];
        let mut state = InjectorState::default();

        // Frames 0 and 1 should not match
        assert!(evaluate(&rules, &mut state, &text_downstream("frame-0"), "C").is_none());
        assert!(evaluate(&rules, &mut state, &text_downstream("frame-1"), "C").is_none());
        // Frame 2 should match
        let action = evaluate(&rules, &mut state, &text_downstream("frame-2"), "C").unwrap();
        assert_eq!(action.trigger_kind, "frame_index_from_downstream");
        assert_eq!(action.frame_index_in, 2);
        // Frame 3 should not match
        assert!(evaluate(&rules, &mut state, &text_downstream("frame-3"), "C").is_none());
    }

    // --- InjectorState defaults ---

    #[test]
    fn injector_state_default_all_zero() {
        let state = InjectorState::default();
        assert_eq!(state.downstream_frame_index, 0);
        assert_eq!(state.upstream_frame_index, 0);
        assert_eq!(state.total_injections_fired, 0);
    }

    // --- direction mismatch ---

    #[test]
    fn direction_mismatch_prevents_firing() {
        let rules = vec![WsFrameInjection {
            trigger: WsFrameTrigger::FirstFrameFromDownstream,
            template: "auth ${credential}".to_string(),
            frame_kind: WsFrameKind::Text,
            consume_trigger: true,
            direction: WsFrameDirection::Upstream, // Rule is for upstream
        }];
        let mut state = InjectorState::default();

        // Downstream frame should not trigger an upstream-direction rule
        assert!(evaluate(&rules, &mut state, &text_downstream("hello"), "CRED").is_none());
    }

    // --- consume_trigger true consumes frame ---

    #[test]
    fn consume_trigger_true_suppresses_forwarding() {
        let rule = ha_rule(); // consume_trigger is true
        let mut state = InjectorState::default();

        let action = evaluate(
            &[rule],
            &mut state,
            &text_downstream(r#"{"type":"auth_required"}"#),
            "TOKEN",
        )
        .expect("should match");
        assert!(!action.forward_original);
    }

    // --- contains_jwt_like_literal ---

    #[test]
    fn jwt_detection_finds_eyj_in_longer_text() {
        assert!(contains_jwt_like_literal("begin eyJhbGciOiJIUzI1NiJ9 end"));
    }

    #[test]
    fn jwt_detection_ignores_eyj_substring_in_non_token() {
        // eyJ not at the start of a token-like segment should still match
        // because the function splits on non-alphanumeric/dot/dash/underscore
        assert!(!contains_jwt_like_literal("not_a_jwt_value"));
    }

    #[test]
    fn jwt_detection_returns_false_for_clean_text() {
        assert!(!contains_jwt_like_literal(
            r#"{"type":"auth","token":"${credential}"}"#
        ));
    }

    // --- multiple rules, first match wins ---

    #[test]
    fn first_matching_rule_wins() {
        let rules = vec![
            WsFrameInjection {
                trigger: WsFrameTrigger::JsonFieldEquals {
                    path: "$.type".to_string(),
                    value: serde_json::json!("auth_required"),
                },
                template: "FIRST ${credential}".to_string(),
                frame_kind: WsFrameKind::Text,
                consume_trigger: true,
                direction: WsFrameDirection::Downstream,
            },
            WsFrameInjection {
                trigger: WsFrameTrigger::JsonFieldEquals {
                    path: "$.type".to_string(),
                    value: serde_json::json!("auth_required"),
                },
                template: "SECOND ${credential}".to_string(),
                frame_kind: WsFrameKind::Text,
                consume_trigger: true,
                direction: WsFrameDirection::Downstream,
            },
        ];
        let mut state = InjectorState::default();

        let action = evaluate(
            &rules,
            &mut state,
            &text_downstream(r#"{"type":"auth_required"}"#),
            "C",
        )
        .expect("match");
        assert_eq!(
            String::from_utf8(action.send_frame.payload).unwrap(),
            "FIRST C"
        );
    }

    // --- credential_sha256_prefix in action ---

    #[test]
    fn action_contains_credential_hash_prefix() {
        let rules = vec![ha_rule()];
        let mut state = InjectorState::default();

        let action = evaluate(
            &rules,
            &mut state,
            &text_downstream(r#"{"type":"auth_required"}"#),
            "secret-token",
        )
        .expect("match");
        assert_eq!(
            action.credential_sha256_prefix,
            credential_hash_prefix("secret-token")
        );
        assert_eq!(action.credential_sha256_prefix.len(), 12);
    }

    // --- json trigger on non-utf8 binary payload ---

    #[test]
    fn json_trigger_on_invalid_utf8_returns_none() {
        let rules = vec![ha_rule()];
        let mut state = InjectorState::default();

        let frame = IncomingFrame {
            direction: WsFrameDirection::Downstream,
            kind: WsFrameKind::Text,
            payload: vec![0xFF, 0xFE, 0xFD],
        };
        assert!(evaluate(&rules, &mut state, &frame, "TOKEN").is_none());
    }
}
