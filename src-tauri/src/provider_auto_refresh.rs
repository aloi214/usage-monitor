//! Per-family quota scheduling preferences, deliberately separate from consent.
//! Missing old-schema entries retain previous behavior. An explicitly malformed
//! value fails closed; unknown or account-specific keys never become settings.
use crate::access_policy::FAMILIES;
use serde_json::{Map, Value};

pub fn enabled(config: &Value, account: &str) -> bool {
    let family = account.split('@').next().unwrap_or(account);
    if family == "hermes" || !FAMILIES.contains(&family) { return false; }
    match config.get("providerAutoRefresh") {
        None => !matches!(family, "claude" | "commandcode"),
        Some(Value::Object(values)) => match values.get(family) {
            None => !matches!(family, "claude" | "commandcode"),
            Some(value) => value.as_bool().unwrap_or(false),
        },
        Some(_) => false,
    }
}

pub fn normalized(config: &Value) -> Value {
    Value::Object(FAMILIES.iter().filter(|family| **family != "hermes")
        .map(|family| ((*family).to_string(), Value::Bool(enabled(config, family))))
        .collect::<Map<_, _>>())
}
