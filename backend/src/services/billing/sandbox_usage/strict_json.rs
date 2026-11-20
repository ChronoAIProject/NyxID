use serde::{
    Deserialize, Deserializer,
    de::{Error, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Value};
use std::fmt;

struct Unique(Value);
impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct JsonVisitor;
        impl<'de> Visitor<'de> for JsonVisitor {
            type Value = Unique;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("JSON without duplicate keys or floating point numbers")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Unique, M::Error> {
                let mut object = Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if object.contains_key(&key) {
                        return Err(M::Error::custom("duplicate_key"));
                    }
                    object.insert(key, map.next_value::<Unique>()?.0);
                }
                Ok(Unique(Value::Object(object)))
            }
            fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<Unique, S::Error> {
                let mut values = Vec::new();
                while let Some(value) = seq.next_element::<Unique>()? {
                    values.push(value.0);
                }
                Ok(Unique(Value::Array(values)))
            }
            fn visit_str<E: Error>(self, s: &str) -> Result<Unique, E> {
                Ok(Unique(s.into()))
            }
            fn visit_string<E: Error>(self, s: String) -> Result<Unique, E> {
                Ok(Unique(s.into()))
            }
            fn visit_bool<E: Error>(self, v: bool) -> Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_unit<E: Error>(self) -> Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
            fn visit_u64<E: Error>(self, v: u64) -> Result<Unique, E> {
                if v > 9_007_199_254_740_991 {
                    return Err(E::custom("unsafe_json_integer"));
                }
                Ok(Unique(v.into()))
            }
            fn visit_i64<E: Error>(self, _: i64) -> Result<Unique, E> {
                Err(E::custom("negative_json_integer"))
            }
            fn visit_f64<E: Error>(self, _: f64) -> Result<Unique, E> {
                Err(E::custom("floating_json_number"))
            }
        }
        deserializer.deserialize_any(JsonVisitor)
    }
}

pub(super) fn parse(bytes: &[u8]) -> Result<Value, &'static str> {
    if bytes.len() > 512 * 1024 {
        return Err("body_too_large");
    }
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let value = Unique::deserialize(&mut decoder).map_err(|_| "invalid_json")?;
    decoder.end().map_err(|_| "trailing_json")?;
    Ok(value.0)
}
