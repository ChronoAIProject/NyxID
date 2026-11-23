//! Temporary JSON owners for secret-bearing authority and transport payloads.
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

#[derive(Serialize)]
#[serde(transparent)]
pub(crate) struct SecretJson(pub(crate) serde_json::Value);

// Build recursively guarded values. An outer Drop alone cannot protect the
// strings parsed before a malformed later element aborts deserialization.
// Parser/library scratch buffers remain outside this application's ownership.
impl<'de> Deserialize<'de> for SecretJson {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Guarded;
        impl<'de> serde::de::Visitor<'de> for Guarded {
            type Value = SecretJson;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a JSON value")
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(SecretJson(serde_json::Value::Null))
            }
            fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                self.visit_unit()
            }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(SecretJson(v.into()))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(SecretJson(v.into()))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(SecretJson(v.into()))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| SecretJson(n.into()))
                    .ok_or_else(|| E::custom("invalid JSON number"))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                self.visit_string(v.to_owned())
            }
            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(SecretJson(v.into()))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let mut guarded = SecretJson(serde_json::Value::Array(Vec::new()));
                while let Some(mut next) = seq.next_element::<SecretJson>()? {
                    guarded
                        .0
                        .as_array_mut()
                        .unwrap()
                        .push(std::mem::take(&mut next.0));
                }
                Ok(guarded)
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut guarded = SecretJson(serde_json::Value::Object(serde_json::Map::new()));
                while let Some(mut key) = map.next_key::<zeroize::Zeroizing<String>>()? {
                    let mut value = map.next_value::<SecretJson>()?;
                    let object = guarded.0.as_object_mut().unwrap();
                    // Wipe both the old key and value if a duplicate replaces
                    // them. Map::insert would otherwise drop the old plaintext.
                    if let Some((mut old_key, mut old_value)) = object.remove_entry(key.as_str()) {
                        old_key.zeroize();
                        clear_json_strings(&mut old_value);
                    }
                    object.insert(std::mem::take(&mut *key), std::mem::take(&mut value.0));
                }
                Ok(guarded)
            }
        }
        deserializer.deserialize_any(Guarded)
    }
}
impl From<serde_json::Value> for SecretJson {
    fn from(value: serde_json::Value) -> Self {
        Self(value)
    }
}
impl std::ops::Deref for SecretJson {
    type Target = serde_json::Value;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for SecretJson {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl std::fmt::Debug for SecretJson {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretJson([REDACTED])")
    }
}
impl Drop for SecretJson {
    fn drop(&mut self) {
        clear_json_strings(&mut self.0);
    }
}
pub(crate) fn clear_json_strings(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(value) => value.zeroize(),
        serde_json::Value::Array(values) => values.iter_mut().for_each(clear_json_strings),
        serde_json::Value::Object(values) => {
            for (mut key, mut value) in std::mem::take(values) {
                key.zeroize();
                clear_json_strings(&mut value);
            }
        }
        _ => {}
    }
}

/// Serialize a secret-bearing transport frame without freeing intermediate
/// plaintext allocations during buffer growth. The first pass counts bytes;
/// the second uses one exact-capacity, zeroizing owner and refuses growth.
pub(crate) fn to_secret_string(
    value: &impl Serialize,
) -> serde_json::Result<zeroize::Zeroizing<String>> {
    use std::io::{Error, Write};
    struct Count(usize);
    impl Write for Count {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .ok_or_else(|| Error::other("JSON frame size overflow"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    struct Exact(zeroize::Zeroizing<Vec<u8>>);
    impl Write for Exact {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.0.capacity() - self.0.len() {
                return Err(Error::other("JSON frame changed during serialization"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    serde_json::to_writer(&mut count, value)?;
    let mut exact = Exact(zeroize::Zeroizing::new(Vec::with_capacity(count.0)));
    serde_json::to_writer(&mut exact, value)?;
    // serde_json writes valid UTF-8; conversion transfers the same allocation.
    let text =
        String::from_utf8(std::mem::take(&mut *exact.0)).expect("JSON encoder produced UTF-8");
    Ok(zeroize::Zeroizing::new(text))
}

#[cfg(test)]
mod tests {
    use super::*;
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
