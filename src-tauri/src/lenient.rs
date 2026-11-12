//! Arguments and schemas made to fit what AI clients really send.
//!
//! Two things were seen with Claude as the client (2026-11-12):
//! - A field that may be left out came out of the schema generator as `"type": ["string", "null"]`. The client
//!   read that as "no type", and sent a code like 1810 as a JSON number, which a text field then refused.
//!   Quoting it did not help: the quotes arrived as part of the text. So every tool's schema names one plain
//!   type for such a field (it stays optional: it is not in `required`).
//! - Whatever the schema says, a client may still send 1810 for a text field or "25" for a number. Before a
//!   tool reads its arguments, a value of the wrong kind that can only mean one thing is given the kind the
//!   schema asks for. Anything else is left alone, so the tool's own refusal still names it.

use rmcp::model::JsonObject;
use serde_json::{Number, Value};

/// `"type": ["string", "null"]` becomes `"type": "string"`, everywhere in the schema.
pub fn plain_optional_types(schema: &mut Value) {
    match schema {
        Value::Object(object) => {
            if let Some(Value::Array(types)) = object.get("type") {
                let named: Vec<&Value> = types
                    .iter()
                    .filter(|t| t.as_str() != Some("null"))
                    .collect();
                if named.len() == 1 && named.len() < types.len() {
                    let only = named[0].clone();
                    object.insert("type".into(), only);
                }
            }
            object.values_mut().for_each(plain_optional_types);
        }
        Value::Array(items) => items.iter_mut().for_each(plain_optional_types),
        _ => {}
    }
}

/// Gives each argument the kind its field asks for, where the client sent the other kind and the meaning is
/// certain: a number or a yes/no for a text field becomes that text, and text that is a whole number or an
/// amount with a dot becomes that number for a number field.
pub fn fit(arguments: &mut JsonObject, schema: &JsonObject) {
    let Some(Value::Object(properties)) = schema.get("properties") else {
        return;
    };
    for (name, value) in arguments.iter_mut() {
        if let Some(field) = properties.get(name) {
            fit_value(value, field, schema, 0);
        }
    }
}

/// How deep a schema may refer on: the tools' schemas go three levels down at most.
const MAX_DEPTH: u8 = 12;

fn fit_value(value: &mut Value, schema: &Value, root: &JsonObject, depth: u8) {
    if depth > MAX_DEPTH {
        return;
    }
    let Some(schema) = resolved(schema, root) else {
        return;
    };
    // A field that may also be empty: the schema of what it holds when it is not.
    for key in ["anyOf", "oneOf"] {
        if let Some(Value::Array(choices)) = schema.get(key) {
            let real: Vec<&Value> = choices
                .iter()
                .filter(|c| c.get("type").and_then(Value::as_str) != Some("null"))
                .collect();
            if let [only] = real[..] {
                fit_value(value, only, root, depth + 1);
            }
            return;
        }
    }
    let asks = |kind: &str| match schema.get("type") {
        Some(Value::String(t)) => t == kind,
        Some(Value::Array(types)) => types.iter().any(|t| t.as_str() == Some(kind)),
        _ => false,
    };
    match value {
        Value::Number(_) | Value::Bool(_)
            if asks("string") && !asks("number") && !asks("integer") && !asks("boolean") =>
        {
            *value = Value::String(value.to_string());
        }
        Value::String(text) if !asks("string") && (asks("integer") || asks("number")) => {
            let text = text.trim();
            let number = if asks("integer") {
                text.parse::<i64>().ok().map(Number::from)
            } else {
                // Only what JSON itself would read as a number: digits with at most one dot, no comma.
                text.parse::<i64>().ok().map(Number::from).or_else(|| {
                    let plain = !text.is_empty()
                        && text
                            .chars()
                            .all(|c| c.is_ascii_digit() || c == '.' || c == '-')
                        && text.chars().any(|c| c.is_ascii_digit());
                    plain
                        .then(|| text.parse::<f64>().ok().and_then(Number::from_f64))
                        .flatten()
                })
            };
            if let Some(number) = number {
                *value = Value::Number(number);
            }
        }
        Value::Object(fields) => {
            if let Some(Value::Object(properties)) = schema.get("properties") {
                for (name, field) in fields.iter_mut() {
                    if let Some(field_schema) = properties.get(name) {
                        fit_value(field, field_schema, root, depth + 1);
                    }
                }
            }
        }
        Value::Array(items) => {
            if let Some(item_schema) = schema.get("items") {
                for item in items.iter_mut() {
                    fit_value(item, item_schema, root, depth + 1);
                }
            }
        }
        _ => {}
    }
}

/// The schema itself, or the one in `$defs` it refers to.
fn resolved<'a>(schema: &'a Value, root: &'a JsonObject) -> Option<&'a Value> {
    match schema.get("$ref").and_then(Value::as_str) {
        None => Some(schema),
        Some(reference) => {
            let name = reference.strip_prefix("#/$defs/")?;
            root.get("$defs")?.get(name)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn object(value: Value) -> JsonObject {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn an_optional_field_names_one_plain_type() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "rekening": {"type": ["string", "null"], "description": "Code"},
                "termijn": {"type": ["integer", "null"], "format": "uint32"},
                "datum": {"type": "string"},
                "beide": {"type": ["string", "integer"]},
                "regels": {"type": ["array", "null"], "items": {"$ref": "#/$defs/Regel"}}
            },
            "$defs": {"Regel": {"type": "object", "properties": {"omschrijving": {"type": ["string", "null"]}}}}
        });
        plain_optional_types(&mut schema);
        assert_eq!(schema["properties"]["rekening"]["type"], "string");
        assert_eq!(schema["properties"]["termijn"]["type"], "integer");
        assert_eq!(schema["properties"]["datum"]["type"], "string");
        // Two real types stay two.
        assert_eq!(
            schema["properties"]["beide"]["type"],
            json!(["string", "integer"])
        );
        assert_eq!(schema["properties"]["regels"]["type"], "array");
        assert_eq!(
            schema["$defs"]["Regel"]["properties"]["omschrijving"]["type"],
            "string"
        );
    }

    fn schema() -> JsonObject {
        object(json!({
            "type": "object",
            "properties": {
                "rekening": {"type": "string"},
                "factuur": {"type": ["string", "null"]},
                "relatie_id": {"type": "integer"},
                "bedrag": {"type": "number"},
                "alles": {"type": "boolean"},
                "soort": {"oneOf": [{"const": "geld_ontvangen"}, {"const": "geld_uitgegeven"}]},
                "betalingen": {"type": "array", "items": {"$ref": "#/$defs/Betaling"}},
                "adres": {"anyOf": [{"$ref": "#/$defs/Adres"}, {"type": "null"}]}
            },
            "$defs": {
                "Betaling": {"type": "object", "properties": {"factuur": {"type": "string"}, "bedrag": {"type": "number"}, "relatie_id": {"type": "integer"}}},
                "Adres": {"type": "object", "properties": {"postcode": {"type": "string"}, "huisnummer": {"type": "string"}}}
            }
        }))
    }

    #[test]
    fn a_code_sent_as_a_number_becomes_the_text_the_field_asks_for() {
        // The call that failed: rekening 1810 as a JSON number.
        let mut args = object(json!({
            "rekening": 1810, "factuur": 2026001, "relatie_id": 74309115, "bedrag": 180.0,
            "betalingen": [{"relatie_id": 74309115, "factuur": 11064766, "bedrag": 180}],
            "adres": {"postcode": "9723 JE", "huisnummer": 2}
        }));
        fit(&mut args, &schema());
        assert_eq!(
            Value::Object(args),
            json!({
                "rekening": "1810", "factuur": "2026001", "relatie_id": 74309115, "bedrag": 180.0,
                "betalingen": [{"relatie_id": 74309115, "factuur": "11064766", "bedrag": 180}],
                "adres": {"postcode": "9723 JE", "huisnummer": "2"}
            })
        );
    }

    #[test]
    fn a_number_sent_as_text_becomes_the_number_only_when_it_can_mean_nothing_else() {
        let mut args = object(json!({
            "relatie_id": " 74309115 ", "bedrag": "180.50",
            "betalingen": [{"relatie_id": "12", "bedrag": "-2", "factuur": "F00002"}]
        }));
        fit(&mut args, &schema());
        assert_eq!(
            Value::Object(args),
            json!({
                "relatie_id": 74309115, "bedrag": 180.5,
                "betalingen": [{"relatie_id": 12, "bedrag": -2, "factuur": "F00002"}]
            })
        );
        // A Dutch comma, words, half a number, a fraction for a whole number: left for the tool to refuse by name.
        for (field, text) in [
            ("bedrag", "180,50"),
            ("bedrag", "veel"),
            ("bedrag", ""),
            ("bedrag", "1e3"),
            ("relatie_id", "12.5"),
            ("relatie_id", "12a"),
        ] {
            let mut args = object(json!({ field: text }));
            fit(&mut args, &schema());
            assert_eq!(args[field], text, "{field}");
        }
    }

    #[test]
    fn what_already_fits_or_is_not_in_the_schema_is_left_alone() {
        let before = json!({
            "rekening": "0130", "alles": true, "soort": "geld_uitgegeven", "onbekend": 5, "bedrag": 2,
            "betalingen": "geen lijst", "adres": null
        });
        let mut args = object(before.clone());
        fit(&mut args, &schema());
        assert_eq!(Value::Object(args), before);
        // A schema without properties, or one that refers to nothing, changes nothing either.
        let mut args = object(json!({"a": 1}));
        fit(&mut args, &object(json!({"type": "object"})));
        assert_eq!(args["a"], 1);
        let mut args = object(json!({"a": 1}));
        fit(
            &mut args,
            &object(json!({"properties": {"a": {"$ref": "#/$defs/Weg"}}})),
        );
        assert_eq!(args["a"], 1);
    }
}
