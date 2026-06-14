use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::errors::UtilsError;

pub fn to_json<T: Serialize>(value: &T) -> Result<String, UtilsError> {
    serde_json::to_string(value)
        .map_err(|e| UtilsError::JsonError(e.to_string()))
}

pub fn from_json<T: DeserializeOwned>(json: &str) -> Result<T, UtilsError> {
    serde_json::from_str(json)
        .map_err(|e| UtilsError::JsonError(e.to_string()))
}

pub fn to_pretty_json<T: Serialize>(value: &T) -> Result<String, UtilsError> {
    serde_json::to_string_pretty(value)
        .map_err(|e| UtilsError::JsonError(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn roundtrip() {
        let mut m: HashMap<String, String> = HashMap::new();
        m.insert("key".to_string(), "val".to_string());
        let json = to_json(&m).unwrap();
        let back: HashMap<String, String> = from_json(&json).unwrap();
        assert_eq!(back["key"], "val");
    }

    #[test]
    fn pretty_json_has_newlines() {
        let m: HashMap<String, u32> = [("a".to_string(), 1u32)].into();
        let pretty = to_pretty_json(&m).unwrap();
        assert!(pretty.contains('\n'));
    }
}
