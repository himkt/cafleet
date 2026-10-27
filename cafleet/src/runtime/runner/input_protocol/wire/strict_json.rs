use crate::runtime::runner::{MAX_CONTROL_FRAME, Result, error};
use serde::de::{IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Number, Value};
use std::fmt;

struct UniqueValue(Value);
impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct UniqueVisitor;
        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = UniqueValue;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("JSON with unique object fields")
            }
            fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }
            fn visit_bool<E: serde::de::Error>(
                self,
                value: bool,
            ) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::Bool(value)))
            }
            fn visit_i64<E: serde::de::Error>(
                self,
                value: i64,
            ) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::Number(value.into())))
            }
            fn visit_u64<E: serde::de::Error>(
                self,
                value: u64,
            ) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::Number(value.into())))
            }
            fn visit_f64<E: serde::de::Error>(
                self,
                value: f64,
            ) -> std::result::Result<Self::Value, E> {
                Number::from_f64(value)
                    .map(|number| UniqueValue(Value::Number(number)))
                    .ok_or_else(|| E::custom("finite JSON number required"))
            }
            fn visit_str<E: serde::de::Error>(
                self,
                value: &str,
            ) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(value.into())))
            }
            fn visit_string<E: serde::de::Error>(
                self,
                value: String,
            ) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(value)))
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(UniqueValue(value)) = sequence.next_element()? {
                    values.push(value);
                }
                Ok(UniqueValue(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut entries: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut object = Map::new();
                while let Some(key) = entries.next_key::<String>()? {
                    if object.contains_key(&key) {
                        return Err(serde::de::Error::custom("duplicate JSON field"));
                    }
                    let UniqueValue(value) = entries.next_value()?;
                    object.insert(key, value);
                }
                Ok(UniqueValue(Value::Object(object)))
            }
        }
        deserializer.deserialize_any(UniqueVisitor)
    }
}

pub(super) fn decode(body: &[u8]) -> Result<Value> {
    super::super::super::ensure(
        body.len() <= MAX_CONTROL_FRAME,
        "control frame exceeds size limit",
    )?;
    serde_json::from_slice::<UniqueValue>(body)
        .map(|value| value.0)
        .map_err(|_| error("malformed or duplicate control JSON fields"))
}

struct InputRoute(bool);
impl<'de> Deserialize<'de> for InputRoute {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct RouteVisitor;
        impl<'de> Visitor<'de> for RouteVisitor {
            type Value = InputRoute;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a control request object")
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut entries: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut input = false;
                while let Some(key) = entries.next_key::<String>()? {
                    if key == "kind" {
                        let value: Value = entries.next_value()?;
                        input |= value.as_str().is_some_and(|kind| {
                            matches!(
                                kind,
                                "submit_prompt"
                                    | "submit_exec"
                                    | "permission_inspect"
                                    | "permission_respond"
                                    | "receipt"
                            )
                        });
                    } else {
                        entries.next_value::<IgnoredAny>()?;
                    }
                }
                Ok(InputRoute(input))
            }
        }
        deserializer.deserialize_map(RouteVisitor)
    }
}

pub(super) fn routes_input(body: &[u8]) -> Result<bool> {
    super::super::super::ensure(
        body.len() <= MAX_CONTROL_FRAME,
        "control frame exceeds size limit",
    )?;
    serde_json::from_slice::<InputRoute>(body)
        .map(|route| route.0)
        .map_err(|_| error("malformed control request object"))
}
