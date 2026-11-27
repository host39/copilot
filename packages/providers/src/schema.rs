//! Tool input schemas as both wire protocols take them.

use serde_json::{Map, Value};

/// `schema` with `type: "object"` and `properties`: Chat Completions servers reject anything
/// else, and the Messages API requires an object schema. A schema that is not a JSON object
/// becomes an empty object schema; a schema of another `type` is passed on as it is.
pub(crate) fn object_schema(schema: &Value) -> Value {
    let mut schema = match schema {
        Value::Object(schema) => schema.clone(),
        _ => Map::new(),
    };
    schema
        .entry("type")
        .or_insert_with(|| Value::from("object"));
    if schema.get("type") == Some(&Value::from("object")) {
        schema
            .entry("properties")
            .or_insert_with(|| Value::Object(Map::new()));
    }
    Value::Object(schema)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn completes_object_schemas() {
        assert_eq!(
            object_schema(&json!(null)),
            json!({ "type": "object", "properties": {} })
        );
        assert_eq!(
            object_schema(&json!({ "required": ["a"] })),
            json!({ "type": "object", "properties": {}, "required": ["a"] })
        );
        let full = json!({ "type": "object", "properties": { "a": { "type": "string" } } });
        assert_eq!(object_schema(&full), full);
    }
}
