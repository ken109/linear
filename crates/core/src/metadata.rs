//! Attachment metadata: the flat `key -> string | number` object Linear lets
//! an attachment carry (`AttachmentCreateInput.metadata`).
//!
//! Linear's own type is the free-form `JSONObject` scalar, and attachments that
//! integrations create (GitHub, Slack, ...) hold nested objects in it. What the
//! CLI *writes* is restricted to what Linear documents ("string and number
//! values") and is validated here before anything is sent. What it *reads*
//! stays a plain JSON object, so nothing an integration stored is lost.

use crate::schema;
use serde::{Serialize, Serializer};
use serde_json::{Map, Number, Value};
use std::collections::BTreeMap;

cynic::impl_scalar!(AttachmentMetadata, schema::JSONObject);
cynic::impl_scalar!(Map<String, Value>, schema::JSONObject);

/// A metadata value: a string or a number, nothing nested.
#[derive(Debug, Clone, PartialEq)]
pub enum MetaValue {
    String(String),
    Number(Number),
}

impl MetaValue {
    /// Parse the right-hand side of `--meta key=value`.
    ///
    /// A value written like a JSON number (`42`, `-1`, `3.5`, `1e3`) is a
    /// number. Everything else is a string, including `007`, `+1`, `.5`,
    /// `NaN` and integers too large to hold exactly (an id must not be
    /// rounded). The prefix `str:` forces a string: `str:123` is the text
    /// `123`, and `str:str:1` is the text `str:1`.
    pub fn parse(raw: &str) -> Self {
        if let Some(text) = raw.strip_prefix("str:") {
            return Self::String(text.to_owned());
        }
        match parse_number(raw) {
            Some(n) => Self::Number(n),
            None => Self::String(raw.to_owned()),
        }
    }

    fn to_json(&self) -> Value {
        match self {
            Self::String(s) => Value::String(s.clone()),
            Self::Number(n) => Value::Number(n.clone()),
        }
    }

    /// Is this the same value as one read back from Linear? Numbers compare by
    /// value (`1` and `1.0` are the same), so a number Linear normalizes does
    /// not look like a change.
    fn same_as(&self, stored: &Value) -> bool {
        match (self, stored) {
            (Self::String(a), Value::String(b)) => a == b,
            (Self::Number(a), Value::Number(b)) => {
                if a.is_f64() || b.is_f64() {
                    a.as_f64() == b.as_f64()
                } else {
                    a == b
                }
            }
            _ => false,
        }
    }
}

/// JSON number grammar only, and exact: an integer that does not fit 64 bits
/// is not a number here.
fn parse_number(raw: &str) -> Option<Number> {
    let digits = raw.strip_prefix('-').unwrap_or(raw);
    let int_end = digits
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(digits.len());
    let int = &digits[..int_end];
    if int.is_empty() || (int.len() > 1 && int.starts_with('0')) {
        return None;
    }
    let rest = &digits[int_end..];
    let is_integer = rest.is_empty();
    if !is_integer && !valid_fraction_and_exponent(rest) {
        return None;
    }
    let number: Number = serde_json::from_str(raw).ok()?;
    if is_integer && !(number.is_i64() || number.is_u64()) {
        return None;
    }
    if number.as_f64().is_some_and(|f| !f.is_finite()) {
        return None;
    }
    Some(number)
}

fn valid_fraction_and_exponent(rest: &str) -> bool {
    let (fraction, exponent) = match rest.find(['e', 'E']) {
        Some(at) => (&rest[..at], Some(&rest[at + 1..])),
        None => (rest, None),
    };
    if !fraction.is_empty() {
        let Some(frac) = fraction.strip_prefix('.') else {
            return false;
        };
        if frac.is_empty() || !frac.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
    }
    match exponent {
        None => true,
        Some(e) => {
            let e = e.strip_prefix(['+', '-']).unwrap_or(e);
            !e.is_empty() && e.bytes().all(|b| b.is_ascii_digit())
        }
    }
}

/// Why metadata cannot be sent.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MetadataError {
    #[error("metadata keys cannot be empty")]
    EmptyKey,
    #[error("metadata key {0:?} is given twice")]
    DuplicateKey(String),
    #[error("--meta needs key=value, got {0:?}")]
    NotKeyValue(String),
    #[error("metadata value for {key:?} must be a string or a number, not {found}")]
    BadValue { key: String, found: &'static str },
}

/// A validated, flat metadata object: non-empty keys, string or number values.
///
/// Serializes as a JSON object (keys in order), which is what Linear's
/// `JSONObject` scalar carries.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AttachmentMetadata(BTreeMap<String, MetaValue>);

impl AttachmentMetadata {
    /// Build from `key=value` arguments (`--meta`). A repeated key is an error
    /// rather than a silent override.
    pub fn from_pairs<S: AsRef<str>>(pairs: &[S]) -> Result<Self, MetadataError> {
        let mut meta = Self::default();
        for pair in pairs {
            let pair = pair.as_ref();
            let (key, value) = pair
                .split_once('=')
                .ok_or_else(|| MetadataError::NotKeyValue(pair.to_owned()))?;
            meta.insert(key.trim(), MetaValue::parse(value))?;
        }
        Ok(meta)
    }

    /// Validate a JSON object: every key non-empty, every value a string or a
    /// number. Nested objects, arrays, booleans and `null` are refused.
    pub fn from_json(object: &Map<String, Value>) -> Result<Self, MetadataError> {
        let mut meta = Self::default();
        for (key, value) in object {
            let value = match value {
                Value::String(s) => MetaValue::String(s.clone()),
                Value::Number(n) => MetaValue::Number(n.clone()),
                other => {
                    return Err(MetadataError::BadValue {
                        key: key.clone(),
                        found: json_kind(other),
                    })
                }
            };
            meta.insert(key, value)?;
        }
        Ok(meta)
    }

    fn insert(&mut self, key: &str, value: MetaValue) -> Result<(), MetadataError> {
        if key.is_empty() {
            return Err(MetadataError::EmptyKey);
        }
        if self.0.insert(key.to_owned(), value).is_some() {
            return Err(MetadataError::DuplicateKey(key.to_owned()));
        }
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn get(&self, key: &str) -> Option<&MetaValue> {
        self.0.get(key)
    }

    /// The value of `key` when it is a string.
    pub fn get_str(&self, key: &str) -> Option<&str> {
        match self.0.get(key) {
            Some(MetaValue::String(s)) => Some(s),
            _ => None,
        }
    }

    pub fn to_json(&self) -> Map<String, Value> {
        self.0
            .iter()
            .map(|(k, v)| (k.clone(), v.to_json()))
            .collect()
    }

    /// Does an attachment that stores `stored` already hold exactly this
    /// metadata? Keys must match both ways: Linear replaces the object on an
    /// upsert, so a stored key that is not wanted is a difference too.
    pub fn matches(&self, stored: &Map<String, Value>) -> bool {
        self.0.len() == stored.len()
            && self
                .0
                .iter()
                .all(|(k, v)| stored.get(k).is_some_and(|s| v.same_as(s)))
    }
}

fn json_kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "a nested object",
    }
}

impl Serialize for AttachmentMetadata {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.to_json().serialize(s)
    }
}
