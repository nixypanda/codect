// A small JSON Schema validator for the subset used by
// `docs/schema/ownai.show.v1.json`.
//
// The schema is validated with this in-crate checker rather than a third-party
// validator so the test suite adds no dependency to the build graph. The
// supported keywords are exactly the ones the schema uses: `$ref` into
// `$defs`, `type` (including a union of types), `required`, `properties`,
// `additionalProperties` (boolean), `items`, `enum`, `const`, and `minimum`.
// An unsupported keyword in the schema is ignored, so the schema is kept
// within this subset deliberately.

use serde_json::Value;

pub fn validate(schema: &Value, instance: &Value) -> Result<(), String> {
    check(schema, schema, instance, "$")
}

fn check(root: &Value, schema: &Value, instance: &Value, path: &str) -> Result<(), String> {
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        let target = resolve(root, reference)
            .ok_or_else(|| format!("{path}: unresolved $ref `{reference}`"))?;
        return check(root, target, instance, path);
    }

    if let Some(alternatives) = schema.get("oneOf").and_then(Value::as_array) {
        let matches = alternatives
            .iter()
            .filter(|candidate| check(root, candidate, instance, path).is_ok())
            .count();
        if matches != 1 {
            return Err(format!(
                "{path}: expected exactly one matching oneOf alternative, found {matches}"
            ));
        }
    }

    if let Some(constant) = schema.get("const")
        && instance != constant
    {
        return Err(format!(
            "{path}: expected const {constant}, found {instance}"
        ));
    }

    if let Some(values) = schema.get("enum").and_then(Value::as_array)
        && !values.contains(instance)
    {
        return Err(format!("{path}: {instance} is not one of {values:?}"));
    }

    if let Some(type_spec) = schema.get("type")
        && !type_matches(type_spec, instance)
    {
        return Err(format!(
            "{path}: expected type {type_spec}, found {instance}"
        ));
    }

    if let Some(minimum) = schema.get("minimum").and_then(Value::as_f64)
        && let Some(number) = instance.as_f64()
        && number < minimum
    {
        return Err(format!("{path}: {number} is below the minimum {minimum}"));
    }

    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        let object = instance
            .as_object()
            .ok_or_else(|| format!("{path}: `required` needs an object, found {instance}"))?;
        for key in required {
            let key = key
                .as_str()
                .ok_or_else(|| format!("{path}: non-string `required` entry"))?;
            if !object.contains_key(key) {
                return Err(format!("{path}: missing required property `{key}`"));
            }
        }
    }

    let properties = schema.get("properties").and_then(Value::as_object);
    if let (Some(object), Some(properties)) = (instance.as_object(), properties) {
        for (key, subschema) in properties {
            if let Some(value) = object.get(key) {
                check(root, subschema, value, &format!("{path}.{key}"))?;
            }
        }
    }

    if schema.get("additionalProperties") == Some(&Value::Bool(false))
        && let (Some(object), Some(properties)) = (instance.as_object(), properties)
    {
        for key in object.keys() {
            if !properties.contains_key(key) {
                return Err(format!("{path}: unexpected property `{key}`"));
            }
        }
    }

    if let Some(items) = schema.get("items").and_then(Value::as_object)
        && let Some(array) = instance.as_array()
    {
        for (index, value) in array.iter().enumerate() {
            check(
                root,
                &Value::Object(items.clone()),
                value,
                &format!("{path}[{index}]"),
            )?;
        }
    }

    Ok(())
}

fn resolve<'a>(root: &'a Value, reference: &str) -> Option<&'a Value> {
    let name = reference.strip_prefix("#/$defs/")?;
    root.get("$defs")?.get(name)
}

fn type_matches(type_spec: &Value, instance: &Value) -> bool {
    match type_spec {
        Value::String(name) => single_type_matches(name, instance),
        Value::Array(names) => names
            .iter()
            .filter_map(Value::as_str)
            .any(|name| single_type_matches(name, instance)),
        _ => true,
    }
}

fn single_type_matches(name: &str, instance: &Value) -> bool {
    match name {
        "object" => instance.is_object(),
        "array" => instance.is_array(),
        "string" => instance.is_string(),
        "integer" => instance.is_i64() || instance.is_u64(),
        "number" => instance.is_number(),
        "boolean" => instance.is_boolean(),
        "null" => instance.is_null(),
        _ => false,
    }
}
