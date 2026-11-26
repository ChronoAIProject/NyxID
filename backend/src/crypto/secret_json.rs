pub(crate) use nyxid_service_adapters::secret_json::{
    SecretJson, clear_json_strings, to_secret_string,
};

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;
    #[test]
    fn guarded_json_handles_nested_duplicates_and_malformed_later_fields() {
        let text = r#"{"credential":"first-secret","nested":[{"key":"nested-secret"}],"credential":"successor-secret"}"#;
        let value: SecretJson = serde_json::from_str(text).unwrap();
        assert_eq!(value["credential"], "successor-secret");
        assert_eq!(
            serde_json::to_value(&value).unwrap(),
            serde_json::from_str::<serde_json::Value>(text).unwrap()
        );
        assert!(!format!("{value:?}").contains("secret"));
        for text in [
            r#"{"credential":"distinctive-secret","nested":[{"key":"nested-secret"},"#,
            r#"{"distinctive-key":"secret","later":"#,
        ] {
            let error = serde_json::from_str::<SecretJson>(text).unwrap_err();
            assert!(!error.to_string().contains("distinctive"));
            assert!(!error.to_string().contains("nested-secret"));
        }
    }
    #[test]
    fn transport_json_preserves_bytes_and_rejects_second_pass_growth() {
        let payload = SecretJson(
            serde_json::json!({"header":"distinctive-secret\nλ", "nested":["private-key"]}),
        );
        assert_eq!(
            to_secret_string(&payload).unwrap().as_str(),
            serde_json::to_string(&payload).unwrap()
        );
        struct Growing(std::cell::Cell<bool>);
        impl Serialize for Growing {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let second = self.0.replace(true);
                serializer.serialize_str(if second {
                    "distinctive-secret-larger-second-pass"
                } else {
                    "a"
                })
            }
        }
        let error = to_secret_string(&Growing(std::cell::Cell::new(false))).unwrap_err();
        assert!(!error.to_string().contains("distinctive-secret"));
    }
}
