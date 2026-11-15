//! Select the completed answer without prepending intermediate assistant messages.
use serde_json::Value;

pub(crate) fn final_text(output: &Value) -> Result<String, ()> {
    let messages: Vec<_> = output
        .as_array()
        .ok_or(())?
        .iter()
        .filter(|item| item["type"] == "message" && item["role"] == "assistant")
        .collect();
    let finals: Vec<_> = messages
        .iter()
        .copied()
        .filter(|item| item["phase"] == "final_answer")
        .collect();
    // Older responses omit phase and may split one answer over several
    // messages. Preserve their complete output: position alone cannot tell
    // progress from answer content without losing valid legacy text.
    let selected = if finals.is_empty() {
        messages
            .into_iter()
            .filter(|item| item["phase"].is_null())
            .collect()
    } else {
        finals
    };
    selected
        .into_iter()
        .flat_map(|item| item["content"].as_array().into_iter().flatten())
        .filter(|part| part["type"] == "output_text")
        .map(|part| part["text"].as_str().ok_or(()))
        .collect::<Result<Vec<_>, _>>()
        .map(|parts| parts.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn message(phase: Value, text: &str) -> Value {
        json!({"type":"message","role":"assistant","phase":phase,
            "content":[{"type":"output_text","text":text}]})
    }

    #[test]
    fn explicit_final_survives_interleaved_tools_and_later_commentary() {
        let output = json!([
            message(json!("commentary"), "Checking the record."),
            {"type":"mcp_call","output":"private tool output"},
            message(json!("final_answer"), "The verified result."),
            message(json!("commentary"), "Intermediate text must not be delivered.")
        ]);
        assert_eq!(final_text(&output), Ok("The verified result.".into()));
    }

    #[test]
    fn legacy_messages_preserve_all_messages_and_text_parts() {
        let mut answer = message(Value::Null, "First part.");
        answer["content"].as_array_mut().unwrap().push(json!({
            "type":"output_text","text":"Second part."
        }));
        let output = json!([message(Value::Null, "First message."), answer]);
        assert_eq!(
            final_text(&output),
            Ok("First message.\n\nFirst part.\n\nSecond part.".into())
        );
    }

    #[test]
    fn legacy_multipart_output_preserves_every_message_across_continuation_rounds() {
        let output = json!([
            message(Value::Null, "First item"),
            message(Value::Null, &"文".repeat(256)),
        ]);
        assert_eq!(
            final_text(&output),
            Ok(format!("First item\n\n{}", "文".repeat(256)))
        );
    }

    #[test]
    fn explicit_final_preserves_multiple_answer_messages_and_cjk() {
        let output = json!([
            message(json!("commentary"), "Checking."),
            message(json!("final_answer"), "First item"),
            message(json!("final_answer"), &"文".repeat(256)),
        ]);
        assert_eq!(
            final_text(&output),
            Ok(format!("First item\n\n{}", "文".repeat(256)))
        );
    }

    #[test]
    fn commentary_only_and_unknown_phases_are_not_final_answers() {
        for phase in ["commentary", "future_phase"] {
            assert_eq!(
                final_text(&json!([message(json!(phase), "Progress.")])),
                Ok(String::new())
            );
        }
    }

    #[test]
    fn empty_output_and_non_assistant_messages_do_not_leak_text() {
        assert_eq!(final_text(&json!([])), Ok(String::new()));
        assert_eq!(
            final_text(&json!([{"type":"message","role":"user",
            "content":[{"type":"output_text","text":"private input"}]}])),
            Ok(String::new())
        );
    }

    #[test]
    fn malformed_selected_text_is_rejected() {
        assert!(final_text(&Value::Null).is_err());
        let mut output = json!([message(json!("final_answer"), "answer")]);
        output[0]["content"][0]["text"] = json!(123);
        assert!(final_text(&output).is_err());
    }
}
