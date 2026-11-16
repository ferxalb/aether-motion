//! Typed declarations from `data-composition-variables` and the coercion of defaults and passed
//! values into one resolved value per variable.
//!
//! A declaration is a JSON object with `id`, `type`, `label`, and `default`, plus the fields its
//! type allows. Anything malformed (unknown fields, a default of the wrong type, a value outside
//! `min`/`max`) is an explicit error, never a silently dropped declaration.

use crate::model::VariableKind;
use crate::parse::parse_color;
use anyhow::{anyhow, bail, Result};
use serde_json::{Map, Value};

/// Most declarations one composition may have.
const MAX_VARIABLES: usize = 256;
/// Longest variable id.
const MAX_ID: usize = 64;
/// Number units that are appended to the CSS value; any other `unit` is only a label.
const CSS_UNITS: [&str; 8] = ["px", "%", "deg", "turn", "rad", "grad", "s", "ms"];

/// One validated declaration.
#[derive(Debug)]
pub(super) struct Declaration {
    pub(super) id: String,
    pub(super) kind: VariableKind,
    pub(super) default: Resolved,
    min: Option<f64>,
    max: Option<f64>,
    max_length: Option<usize>,
    /// CSS unit appended to a number's CSS value (`px`, `%`, ...), when `unit` names one.
    css_unit: Option<String>,
    /// Enum option values, in declaration order.
    options: Vec<String>,
}

/// A coerced value: the text form plus typed views for conditions.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Resolved {
    /// What `data-var-text` writes and `data-var-if` compares.
    pub(super) text: String,
    pub(super) number: Option<f64>,
    pub(super) boolean: Option<bool>,
}

/// Shortest decimal form of a finite number; `-0` prints as `0`.
fn format_number(n: f64) -> String {
    format!("{}", n + 0.0)
}

fn kind_of(name: &str) -> Result<VariableKind> {
    Ok(match name {
        "string" => VariableKind::String,
        "number" => VariableKind::Number,
        "color" => VariableKind::Color,
        "boolean" => VariableKind::Boolean,
        "enum" => VariableKind::Enum,
        "image" => VariableKind::Image,
        "font" => VariableKind::Font,
        other => bail!(
            "unknown variable type {other:?}; use string, number, color, boolean, enum, image, or font"
        ),
    })
}

/// Fields each type accepts besides `id`, `type`, `label`, `description`, and `default`.
fn extra_fields(kind: VariableKind) -> &'static [&'static str] {
    match kind {
        VariableKind::String => &["placeholder", "maxLength"],
        VariableKind::Number => &["min", "max", "step", "unit"],
        VariableKind::Color | VariableKind::Image => &["brandRole"],
        VariableKind::Enum => &["options"],
        VariableKind::Boolean | VariableKind::Font => &[],
    }
}

fn check_id(id: &str) -> Result<()> {
    let mut chars = id.chars();
    let valid = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && id.len() <= MAX_ID;
    if !valid {
        bail!(
            "variable id {id:?} must start with a letter or _ and use only letters, digits, _, and - (at most {MAX_ID} characters)"
        );
    }
    Ok(())
}

fn optional_number(object: &Map<String, Value>, key: &str, id: &str) -> Result<Option<f64>> {
    match object.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_f64()
            .map(Some)
            .ok_or_else(|| anyhow!("variable {id}: {key} must be a number")),
    }
}

fn optional_string<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    id: &str,
) -> Result<Option<&'a str>> {
    match object.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_str()
            .map(Some)
            .ok_or_else(|| anyhow!("variable {id}: {key} must be a string")),
    }
}

/// Enum options: plain strings or `{value, label}` objects; non-empty and unique.
fn parse_options(value: Option<&Value>, id: &str) -> Result<Vec<String>> {
    let items = value
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("variable {id}: an enum needs options, an array of values"))?;
    let mut options: Vec<String> = Vec::with_capacity(items.len());
    for item in items {
        let option = match item {
            Value::String(text) => text.clone(),
            Value::Object(fields) => {
                if let Some(key) = fields
                    .keys()
                    .find(|k| !matches!(k.as_str(), "value" | "label"))
                {
                    bail!("variable {id}: unknown option field {key:?} (use value and label)");
                }
                if fields.get("label").is_some_and(|l| !l.is_string()) {
                    bail!("variable {id}: option labels must be strings");
                }
                fields
                    .get("value")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("variable {id}: each option needs a string value"))?
                    .to_string()
            }
            _ => bail!("variable {id}: each option must be a string or {{value, label}}"),
        };
        if options.contains(&option) {
            bail!("variable {id}: option {option:?} is listed twice");
        }
        options.push(option);
    }
    if options.is_empty() {
        bail!("variable {id}: an enum needs at least one option");
    }
    Ok(options)
}

/// Parses the attribute text: a JSON array of declarations with unique ids.
pub(super) fn parse_declarations(raw: &str) -> Result<Vec<Declaration>> {
    let parsed: Value = serde_json::from_str(raw)
        .map_err(|e| anyhow!("data-composition-variables is not valid JSON: {e}"))?;
    let items = parsed.as_array().ok_or_else(|| {
        anyhow!("data-composition-variables must be a JSON array of declarations")
    })?;
    if items.len() > MAX_VARIABLES {
        bail!("data-composition-variables declares more than {MAX_VARIABLES} variables");
    }
    let mut declarations: Vec<Declaration> = Vec::with_capacity(items.len());
    for item in items {
        let declaration = parse_declaration(item)?;
        if declarations.iter().any(|d| d.id == declaration.id) {
            bail!("variable {} is declared twice", declaration.id);
        }
        declarations.push(declaration);
    }
    Ok(declarations)
}

fn parse_declaration(item: &Value) -> Result<Declaration> {
    let object = item
        .as_object()
        .ok_or_else(|| anyhow!("each variable declaration must be a JSON object"))?;
    let id = object
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("a variable declaration needs a string id"))?;
    check_id(id)?;
    let kind = kind_of(
        object
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("variable {id}: needs a string type"))?,
    )
    .map_err(|e| anyhow!("variable {id}: {e}"))?;
    if !object.get("label").is_some_and(Value::is_string) {
        bail!("variable {id}: needs a string label");
    }
    optional_string(object, "description", id)?;
    let extras = extra_fields(kind);
    if let Some(key) = object.keys().find(|k| {
        !matches!(
            k.as_str(),
            "id" | "type" | "label" | "description" | "default"
        ) && !extras.contains(&k.as_str())
    }) {
        let allowed = ["id", "type", "label", "description", "default"]
            .iter()
            .chain(extras)
            .copied()
            .collect::<Vec<_>>()
            .join(", ");
        bail!(
            "variable {id}: unknown field {key:?} for type {} (allowed: {allowed})",
            kind.name()
        );
    }
    let min = optional_number(object, "min", id)?;
    let max = optional_number(object, "max", id)?;
    if let (Some(lo), Some(hi)) = (min, max) {
        if lo > hi {
            bail!("variable {id}: min is greater than max");
        }
    }
    if optional_number(object, "step", id)?.is_some_and(|step| step <= 0.0) {
        bail!("variable {id}: step must be greater than 0");
    }
    optional_string(object, "placeholder", id)?;
    optional_string(object, "brandRole", id)?;
    let max_length = match object.get("maxLength") {
        None => None,
        Some(value) => Some(
            value
                .as_u64()
                .ok_or_else(|| anyhow!("variable {id}: maxLength must be a whole number"))?
                as usize,
        ),
    };
    let css_unit = optional_string(object, "unit", id)?
        .filter(|unit| CSS_UNITS.contains(unit))
        .map(str::to_string);
    let options = if kind == VariableKind::Enum {
        parse_options(object.get("options"), id)?
    } else {
        Vec::new()
    };
    let mut declaration = Declaration {
        id: id.to_string(),
        kind,
        default: Resolved {
            text: String::new(),
            number: None,
            boolean: None,
        },
        min,
        max,
        max_length,
        css_unit,
        options,
    };
    let default = object
        .get("default")
        .ok_or_else(|| anyhow!("variable {id}: needs a default"))?;
    declaration.default = declaration.coerce(default, "default")?;
    Ok(declaration)
}

impl Declaration {
    /// Checks `value` against the declared type and limits; `origin` names it in errors.
    pub(super) fn coerce(&self, value: &Value, origin: &str) -> Result<Resolved> {
        let id = &self.id;
        let text = |text: String| Resolved {
            text,
            number: None,
            boolean: None,
        };
        let wrong = || anyhow!("variable {id}: the {origin} must be {}", self.expected());
        Ok(match self.kind {
            VariableKind::String => {
                let s = value.as_str().ok_or_else(wrong)?;
                if let Some(limit) = self.max_length {
                    if s.chars().count() > limit {
                        bail!("variable {id}: the {origin} is longer than maxLength {limit}");
                    }
                }
                text(s.to_string())
            }
            VariableKind::Number => {
                let n = value.as_f64().ok_or_else(wrong)?;
                if self.min.is_some_and(|lo| n < lo) || self.max.is_some_and(|hi| n > hi) {
                    bail!(
                        "variable {id}: the {origin} {} is outside {}..{}",
                        format_number(n),
                        self.min.map_or("".into(), format_number),
                        self.max.map_or("".into(), format_number)
                    );
                }
                Resolved {
                    text: format_number(n),
                    number: Some(n),
                    boolean: None,
                }
            }
            VariableKind::Color => {
                let s = value.as_str().ok_or_else(wrong)?;
                parse_color(s)
                    .map_err(|e| anyhow!("variable {id}: the {origin} is not a color: {e}"))?;
                text(s.trim().to_string())
            }
            VariableKind::Boolean => {
                let b = value.as_bool().ok_or_else(wrong)?;
                Resolved {
                    text: b.to_string(),
                    number: None,
                    boolean: Some(b),
                }
            }
            VariableKind::Enum => {
                let s = value.as_str().ok_or_else(wrong)?;
                if !self.options.iter().any(|o| o == s) {
                    bail!(
                        "variable {id}: the {origin} {s:?} is not one of its options ({})",
                        self.options.join(", ")
                    );
                }
                text(s.to_string())
            }
            VariableKind::Image => {
                text(object_or_string(value, "url", &["url", "alt"], id, origin)?)
            }
            VariableKind::Font => {
                let name = object_or_string(value, "name", &["name"], id, origin)?;
                if name.trim().is_empty() {
                    bail!("variable {id}: the {origin} font name is empty");
                }
                text(name)
            }
        })
    }

    fn expected(&self) -> &'static str {
        match self.kind {
            VariableKind::String => "a string",
            VariableKind::Number => "a number",
            VariableKind::Color => "a color string",
            VariableKind::Boolean => "true or false",
            VariableKind::Enum => "one of the option values",
            VariableKind::Image => "a local file path or {url}",
            VariableKind::Font => "a font family name or {name}",
        }
    }

    /// The custom property value `--id` takes: the text form, with a CSS unit after numbers.
    pub(super) fn css(&self, value: &Resolved) -> String {
        match &self.css_unit {
            Some(unit) if self.kind == VariableKind::Number => format!("{}{unit}", value.text),
            _ => value.text.clone(),
        }
    }
}

/// A string, or an object whose `key` is a string and whose other fields are in `allowed`.
fn object_or_string(
    value: &Value,
    key: &str,
    allowed: &[&str],
    id: &str,
    origin: &str,
) -> Result<String> {
    match value {
        Value::String(s) => Ok(s.clone()),
        Value::Object(fields) => {
            if let Some(extra) = fields.keys().find(|k| !allowed.contains(&k.as_str())) {
                bail!(
                    "variable {id}: the {origin} has field {extra:?}, which is not supported (allowed: {})",
                    allowed.join(", ")
                );
            }
            if fields.values().any(|v| !v.is_string()) {
                bail!("variable {id}: the fields of the {origin} must be strings");
            }
            fields
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| anyhow!("variable {id}: the {origin} needs a string {key}"))
        }
        _ => bail!("variable {id}: the {origin} must be a string or {{{key}}}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn one(declaration: Value) -> Result<Declaration> {
        parse_declarations(&json!([declaration]).to_string()).map(|mut d| d.remove(0))
    }

    #[test]
    fn types_coerce_and_format() {
        let n = one(json!({"id":"size","type":"number","label":"Size","default":48,"unit":"px","min":8,"max":200})).unwrap();
        assert_eq!(n.default.text, "48");
        assert_eq!(n.css(&n.default), "48px");
        assert!(n.coerce(&json!(4), "value").is_err(), "below min");
        let free = one(json!({"id":"n","type":"number","label":"N","default":2.5,"unit":"items"}))
            .unwrap();
        assert_eq!(
            free.css(&free.default),
            "2.5",
            "a non-CSS unit is only a label"
        );
        assert_eq!(format_number(-0.0), "0");
        let b = one(json!({"id":"on","type":"boolean","label":"On","default":false})).unwrap();
        assert_eq!(
            (b.default.text.as_str(), b.default.boolean),
            ("false", Some(false))
        );
        let e = one(json!({"id":"t","type":"enum","label":"T","default":"b","options":["a",{"value":"b","label":"B"}]})).unwrap();
        assert!(e.coerce(&json!("c"), "value").is_err());
        let i = one(json!({"id":"logo","type":"image","label":"Logo","default":{"url":"logo.png","alt":"x"}})).unwrap();
        assert_eq!(i.default.text, "logo.png");
        let c = one(json!({"id":"accent","type":"color","label":"Accent","default":" #ff0000 "}))
            .unwrap();
        assert_eq!(c.default.text, "#ff0000");
    }

    #[test]
    fn malformed_declarations_are_errors() {
        for (bad, needle) in [
            (
                json!({"id":"a","type":"string","label":"A"}),
                "needs a default",
            ),
            (json!({"id":"a","type":"string","default":"x"}), "label"),
            (
                json!({"id":"1a","type":"string","label":"A","default":"x"}),
                "must start",
            ),
            (
                json!({"id":"a","type":"text","label":"A","default":"x"}),
                "unknown variable type",
            ),
            (
                json!({"id":"a","type":"string","label":"A","default":"x","defualt":1}),
                "unknown field",
            ),
            (
                json!({"id":"a","type":"number","label":"A","default":"3"}),
                "must be a number",
            ),
            (
                json!({"id":"a","type":"number","label":"A","default":5,"max":4}),
                "outside",
            ),
            (
                json!({"id":"a","type":"number","label":"A","default":1,"min":2,"max":1}),
                "min is greater",
            ),
            (
                json!({"id":"a","type":"color","label":"A","default":"nope"}),
                "not a color",
            ),
            (
                json!({"id":"a","type":"enum","label":"A","default":"x","options":[]}),
                "at least one",
            ),
            (
                json!({"id":"a","type":"enum","label":"A","default":"x","options":["x","x"]}),
                "twice",
            ),
            (
                json!({"id":"a","type":"string","label":"A","default":"long","maxLength":2}),
                "maxLength",
            ),
            (
                json!({"id":"a","type":"font","label":"A","default":{"name":"Inter","source":"u"}}),
                "not supported",
            ),
            (
                json!({"id":"a","type":"image","label":"A","default":{"url":"a.png","fit":"cover"}}),
                "not supported",
            ),
        ] {
            let error = format!("{:#}", one(bad.clone()).unwrap_err());
            assert!(error.contains(needle), "{bad}: {error}");
        }
        let twice = json!([
            {"id":"a","type":"boolean","label":"A","default":true},
            {"id":"a","type":"boolean","label":"A","default":true}
        ]);
        assert!(parse_declarations(&twice.to_string()).is_err());
        assert!(parse_declarations("{}").is_err());
    }
}
