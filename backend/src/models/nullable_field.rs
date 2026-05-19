use serde::{Deserialize, Deserializer};

pub fn deserialize<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Ok(Some(Option::<T>::deserialize(deserializer)?))
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Req {
        #[serde(default, deserialize_with = "super::deserialize")]
        platform: Option<Option<String>>,
        #[serde(default, deserialize_with = "super::deserialize")]
        count: Option<Option<u32>>,
    }

    #[test]
    fn field_absent_is_outer_none() {
        let req: Req = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(req.platform, None);
        assert_eq!(req.count, None);
    }

    #[test]
    fn field_null_is_some_none() {
        let req: Req = serde_json::from_str(r#"{"platform": null, "count": null}"#).unwrap();
        assert_eq!(req.platform, Some(None));
        assert_eq!(req.count, Some(None));
    }

    #[test]
    fn field_present_is_some_some() {
        let req: Req = serde_json::from_str(r#"{"platform": "claude-code", "count": 42}"#).unwrap();
        assert_eq!(req.platform, Some(Some("claude-code".to_string())));
        assert_eq!(req.count, Some(Some(42)));
    }
}
