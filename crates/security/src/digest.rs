use serde_json::Value;
use sha2::{Digest, Sha256};

/// Serializes JSON with object keys sorted so equal values hash equally.
pub fn canonical_json(v: &Value) -> String {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let parts: Vec<String> = keys
                .into_iter()
                .map(|k| format!("{}:{}", Value::String(k.clone()), canonical_json(&map[k])))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        Value::Array(items) => format!("[{}]", items.iter().map(canonical_json).collect::<Vec<_>>().join(",")),
        other => other.to_string(),
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn json_digest(v: &Value) -> String {
    sha256_hex(canonical_json(v).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_order_does_not_change_digest() {
        assert_eq!(
            json_digest(&json!({"a":1,"b":[1,{"y":2,"x":1}]})),
            json_digest(&json!({"b":[1,{"x":1,"y":2}],"a":1}))
        );
        assert_ne!(json_digest(&json!({"a":1})), json_digest(&json!({"a":2})));
    }
}
