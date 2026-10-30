//! Validates command arguments against the subset of JSON Schema the command catalog uses.
//! Description and default are annotations.
use super::{Error, Result, Value};

fn invalid_schema() -> Error {
    Error::new("Invalid service command argument schema.")
}
fn invalid_argument(path: &str, reason: &str) -> Error {
    Error::new(format!("Invalid command arguments: {path} {reason}."))
}
fn matches_type(constraint: &Value, value: &Value) -> Result<bool> {
    let types = match constraint {
        Value::String(name) => vec![name.as_str()],
        Value::Array(names) => names
            .iter()
            .map(|name| name.as_str().ok_or_else(invalid_schema))
            .collect::<Result<Vec<_>>>()?,
        _ => return Err(invalid_schema()),
    };
    let mut matches = false;
    for name in types {
        matches |= match name {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            _ => return Err(invalid_schema()),
        };
    }
    Ok(matches)
}
fn validate_object_keyword(
    schema: &Value,
    keyword: &str,
    constraint: &Value,
    value: &Value,
    path: &str,
) -> Result<()> {
    let invalid = |reason: &str| invalid_argument(path, reason);
    match keyword {
        "properties" => {
            let properties = constraint.as_object().ok_or_else(invalid_schema)?;
            if let Some(object) = value.as_object() {
                for (key, nested) in properties {
                    if let Some(value) = object.get(key) {
                        validate(nested, value, &format!("{path}.{key}"))?;
                    }
                }
            }
        }
        "additionalProperties" => {
            let allowed = constraint.as_bool().ok_or_else(invalid_schema)?;
            if !allowed {
                let properties = schema["properties"]
                    .as_object()
                    .ok_or_else(invalid_schema)?;
                if value
                    .as_object()
                    .is_some_and(|object| object.keys().any(|key| !properties.contains_key(key)))
                {
                    return Err(invalid("contains an unknown field"));
                }
            }
        }
        "required" => {
            for required in constraint.as_array().ok_or_else(invalid_schema)? {
                let key = required.as_str().ok_or_else(invalid_schema)?;
                if value
                    .as_object()
                    .is_some_and(|object| !object.contains_key(key))
                {
                    return Err(invalid(&format!("requires {key}")));
                }
            }
        }
        _ => return Err(invalid_schema()),
    }
    Ok(())
}
fn validate_array_keyword(
    keyword: &str,
    constraint: &Value,
    value: &Value,
    path: &str,
) -> Result<()> {
    let invalid = |reason: &str| invalid_argument(path, reason);
    match keyword {
        "items" => {
            if let Some(items) = value.as_array() {
                for item in items {
                    validate(constraint, item, path)?;
                }
            }
        }
        "minItems" | "maxItems" => {
            let limit = constraint.as_u64().ok_or_else(invalid_schema)?;
            if value.as_array().is_some_and(|items| {
                (keyword == "maxItems" && items.len() as u64 > limit)
                    || (keyword == "minItems" && (items.len() as u64) < limit)
            }) {
                return Err(invalid("has an invalid number of items"));
            }
        }
        "uniqueItems" => {
            let unique = constraint.as_bool().ok_or_else(invalid_schema)?;
            if unique
                && value.as_array().is_some_and(|items| {
                    items
                        .iter()
                        .enumerate()
                        .any(|(index, item)| items[..index].contains(item))
                })
            {
                return Err(invalid("contains duplicate items"));
            }
        }
        _ => return Err(invalid_schema()),
    }
    Ok(())
}
pub(super) fn validate(schema: &Value, value: &Value, path: &str) -> Result<()> {
    let invalid = |reason: &str| invalid_argument(path, reason);
    let definition = schema.as_object().ok_or_else(invalid_schema)?;
    for (keyword, constraint) in definition {
        match keyword.as_str() {
            "oneOf" => {
                let alternatives = constraint
                    .as_array()
                    .filter(|items| !items.is_empty())
                    .ok_or_else(invalid_schema)?;
                if alternatives
                    .iter()
                    .filter(|nested| validate(nested, value, path).is_ok())
                    .count()
                    != 1
                {
                    return Err(invalid("must match exactly one supported form"));
                }
            }
            "type" => {
                if !matches_type(constraint, value)? {
                    return Err(invalid("has the wrong type"));
                }
            }
            "properties" | "additionalProperties" | "required" => {
                validate_object_keyword(schema, keyword, constraint, value, path)?;
            }
            "enum" => {
                if !constraint
                    .as_array()
                    .ok_or_else(invalid_schema)?
                    .contains(value)
                {
                    return Err(invalid("has an unsupported value"));
                }
            }
            "minimum" | "maximum" => {
                let bound = constraint.as_f64().ok_or_else(invalid_schema)?;
                if let Some(number) = value.as_f64() {
                    if (keyword == "minimum" && number < bound)
                        || (keyword == "maximum" && number > bound)
                    {
                        return Err(invalid("is outside the allowed range"));
                    }
                }
            }
            "maxLength" => {
                let limit = constraint.as_u64().ok_or_else(invalid_schema)?;
                if value
                    .as_str()
                    .is_some_and(|text| text.chars().count() as u64 > limit)
                {
                    return Err(invalid("is too long"));
                }
            }
            "pattern" => {
                let pattern = regex::Regex::new(constraint.as_str().ok_or_else(invalid_schema)?)
                    .map_err(|_| invalid_schema())?;
                if value.as_str().is_some_and(|text| !pattern.is_match(text)) {
                    return Err(invalid("has an invalid format"));
                }
            }
            "items" | "minItems" | "maxItems" | "uniqueItems" => {
                validate_array_keyword(keyword, constraint, value, path)?;
            }
            "description" | "default" => {}
            _ => return Err(invalid_schema()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
