// Duplicate of backend/src/services/ws_frame_injector.rs — keep in sync manually. See issue #493.

use serde::Deserialize;

pub const MAX_INJECTIONS_PER_CONNECTION: usize = 8;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WsFrameTrigger {
    FirstFrameFromDownstream,
    JsonFieldEquals {
        path: String,
        value: serde_json::Value,
    },
    FrameIndexFromDownstream {
        index: usize,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WsFrameKind {
    Text,
    Binary,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WsFrameDirection {
    Downstream,
    Upstream,
}

#[derive(Clone, Debug, Deserialize)]
pub struct WsFrameInjection {
    pub trigger: WsFrameTrigger,
    pub template: String,
    #[serde(default = "default_frame_kind_text")]
    pub frame_kind: WsFrameKind,
    #[serde(default = "default_true")]
    pub consume_trigger: bool,
    #[serde(default = "default_direction_downstream")]
    pub direction: WsFrameDirection,
}

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
        return Some(InjectionAction {
            send_frame: WsFrame {
                kind: rule.frame_kind,
                payload: rule
                    .template
                    .replace("${credential}", credential)
                    .into_bytes(),
            },
            forward_original: !rule.consume_trigger,
            trigger_kind,
            frame_index_in,
        });
    }

    None
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

fn default_frame_kind_text() -> WsFrameKind {
    WsFrameKind::Text
}

fn default_direction_downstream() -> WsFrameDirection {
    WsFrameDirection::Downstream
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_rule(
        trigger: WsFrameTrigger,
        template: &str,
        direction: WsFrameDirection,
    ) -> WsFrameInjection {
        WsFrameInjection {
            trigger,
            template: template.to_string(),
            frame_kind: WsFrameKind::Text,
            consume_trigger: true,
            direction,
        }
    }

    fn make_text_frame(direction: WsFrameDirection, payload: &str) -> IncomingFrame {
        IncomingFrame {
            direction,
            kind: WsFrameKind::Text,
            payload: payload.as_bytes().to_vec(),
        }
    }

    #[test]
    fn evaluate_first_frame_trigger_fires_on_first_downstream() {
        let rules = vec![make_rule(
            WsFrameTrigger::FirstFrameFromDownstream,
            r#"{"auth":"${credential}"}"#,
            WsFrameDirection::Downstream,
        )];
        let mut state = InjectorState::default();
        let frame = make_text_frame(WsFrameDirection::Downstream, "hello");

        let action = evaluate(&rules, &mut state, &frame, "secret123").unwrap();
        assert_eq!(action.trigger_kind, "first_frame_from_downstream");
        assert_eq!(action.frame_index_in, 0);
        assert!(!action.forward_original);
        assert_eq!(
            String::from_utf8(action.send_frame.payload.clone()).unwrap(),
            r#"{"auth":"secret123"}"#
        );
    }

    #[test]
    fn evaluate_first_frame_trigger_does_not_fire_on_second() {
        let rules = vec![make_rule(
            WsFrameTrigger::FirstFrameFromDownstream,
            "inject",
            WsFrameDirection::Downstream,
        )];
        let mut state = InjectorState::default();
        let frame = make_text_frame(WsFrameDirection::Downstream, "hello");

        let _ = evaluate(&rules, &mut state, &frame, "cred");
        let second = evaluate(&rules, &mut state, &frame, "cred");
        assert!(second.is_none());
    }

    #[test]
    fn evaluate_frame_index_trigger() {
        let rules = vec![make_rule(
            WsFrameTrigger::FrameIndexFromDownstream { index: 2 },
            "injected",
            WsFrameDirection::Downstream,
        )];
        let mut state = InjectorState::default();
        let frame = make_text_frame(WsFrameDirection::Downstream, "data");

        assert!(evaluate(&rules, &mut state, &frame, "c").is_none());
        assert!(evaluate(&rules, &mut state, &frame, "c").is_none());
        let action = evaluate(&rules, &mut state, &frame, "c").unwrap();
        assert_eq!(action.trigger_kind, "frame_index_from_downstream");
        assert_eq!(action.frame_index_in, 2);
    }

    #[test]
    fn evaluate_json_field_equals_trigger() {
        let rules = vec![make_rule(
            WsFrameTrigger::JsonFieldEquals {
                path: "$.type".to_string(),
                value: json!("auth_required"),
            },
            r#"{"token":"${credential}"}"#,
            WsFrameDirection::Downstream,
        )];
        let mut state = InjectorState::default();
        let frame = make_text_frame(
            WsFrameDirection::Downstream,
            r#"{"type":"auth_required","msg":"hello"}"#,
        );

        let action = evaluate(&rules, &mut state, &frame, "my-key").unwrap();
        assert_eq!(action.trigger_kind, "json_field_equals");
        assert_eq!(
            String::from_utf8(action.send_frame.payload.clone()).unwrap(),
            r#"{"token":"my-key"}"#
        );
    }

    #[test]
    fn evaluate_respects_max_injections_limit() {
        let rules = vec![make_rule(
            WsFrameTrigger::FrameIndexFromDownstream { index: 0 },
            "inject",
            WsFrameDirection::Downstream,
        )];
        let mut state = InjectorState {
            downstream_frame_index: 0,
            upstream_frame_index: 0,
            total_injections_fired: MAX_INJECTIONS_PER_CONNECTION,
        };
        let frame = make_text_frame(WsFrameDirection::Downstream, "data");

        assert!(evaluate(&rules, &mut state, &frame, "c").is_none());
    }

    #[test]
    fn get_json_path_value_traverses_nested_objects() {
        let val = json!({"a": {"b": {"c": 42}}});
        assert_eq!(get_json_path_value(&val, "$.a.b.c"), Some(&json!(42)));
        assert!(get_json_path_value(&val, "$.a.x").is_none());
        assert!(get_json_path_value(&val, "no_prefix").is_none());
        assert!(get_json_path_value(&val, "$.").is_none());
    }
}
