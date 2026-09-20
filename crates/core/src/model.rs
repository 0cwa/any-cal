use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use std::fmt;

/// Split an iCalendar/vCard property head while respecting quoted parameter
/// values.  Commas and semicolons inside quoted values are data, not
/// separators.  Quotes are syntax and are removed from the stored value.
pub(crate) fn parse_property_head(
    left: &str,
) -> Result<(String, BTreeMap<String, Vec<String>>), String> {
    let pieces = split_unquoted(left, ';')?;
    let name = pieces
        .first()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "empty property name".to_owned())?
        .to_ascii_uppercase();
    let mut params = BTreeMap::new();
    for param in pieces.iter().skip(1) {
        let (key, raw) = param
            .split_once('=')
            .ok_or_else(|| format!("invalid parameter {param:?}"))?;
        if key.is_empty() || raw.is_empty() {
            return Err("empty parameter".to_owned());
        }
        let values = split_unquoted(raw, ',')?
            .into_iter()
            .map(|value| unquote(&value))
            .collect::<Vec<_>>();
        if values.iter().any(|value| value.is_empty()) {
            return Err("empty parameter value".to_owned());
        }
        params
            .entry(key.to_ascii_uppercase())
            .or_insert_with(Vec::new)
            .extend(values);
    }
    Ok((name, params))
}

pub(crate) fn serialize_parameter_value(value: &str) -> String {
    if value
        .chars()
        .any(|character| matches!(character, ',' | ';' | ':' | '"' | ' ' | '\t'))
    {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        value.to_owned()
    }
}

/// Calendar list/structured properties use commas and (for RRULE) semicolons
/// as syntax.  Escaping those delimiters would make an otherwise valid
/// recurrence opaque text on the wire.
pub(crate) fn serialize_property_value(name: &str, value: &str) -> String {
    if matches!(
        name,
        "CATEGORIES" | "EXDATE" | "FREEBUSY" | "RDATE" | "RRULE"
    ) {
        value.to_owned()
    } else {
        value
            .replace('\\', "\\\\")
            .replace('\n', "\\n")
            .replace(',', "\\,")
            .replace(';', "\\;")
    }
}

fn split_unquoted(input: &str, delimiter: char) -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for character in input.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' && quoted {
            current.push(character);
            escaped = true;
            continue;
        }
        if character == '"' {
            quoted = !quoted;
            current.push(character);
        } else if character == delimiter && !quoted {
            result.push(current);
            current = String::new();
        } else {
            current.push(character);
        }
    }
    if quoted {
        return Err("unterminated quoted parameter value".to_owned());
    }
    result.push(current);
    Ok(result)
}

fn unquote(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        let mut decoded = String::new();
        let mut chars = value[1..value.len() - 1].chars();
        while let Some(character) = chars.next() {
            if character == '\\' {
                if let Some(escaped) = chars.next() {
                    decoded.push(escaped);
                } else {
                    decoded.push('\\');
                }
            } else {
                decoded.push(character);
            }
        }
        decoded
    } else {
        value.to_owned()
    }
}

macro_rules! identity {
    ($name:ident, $empty:literal) => {
        #[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, &'static str> {
                let value = value.into();
                if value.trim().is_empty() {
                    Err($empty)
                } else {
                    Ok(Self(value))
                }
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl TryFrom<String> for $name {
            type Error = &'static str;
            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }
        impl TryFrom<&str> for $name {
            type Error = &'static str;
            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

identity!(
    CollectionId,
    "collection ID must not be empty or whitespace"
);
identity!(ResourceId, "resource ID must not be empty or whitespace");
identity!(
    AnytypeObjectId,
    "Anytype object ID must not be empty or whitespace"
);
identity!(DavUid, "DAV UID must not be empty or whitespace");

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Collection {
    pub id: CollectionId,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Resource {
    pub id: ResourceId,
    pub kind: DavKind,
    pub anytype_object_id: AnytypeObjectId,
    pub dav_uid: DavUid,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DavKind {
    Contact,
    ContactGroup,
    Task,
    Event,
}

/// One occurrence of a DAV property. Parameters belong to the occurrence,
/// rather than to the property as a whole (e.g. each TEL can have its own
/// TYPE and PREF).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Occurrence {
    pub value: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, Vec<String>>,
}

impl Occurrence {
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            params: BTreeMap::new(),
        }
    }
}

/// A DAV document represented without assuming a fixed set of fields.
/// Property names are keys and repeated occurrences retain their order.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct StructuredDocument {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, Vec<Occurrence>>,
}

impl StructuredDocument {
    pub fn insert(&mut self, name: impl Into<String>, occurrences: Vec<Occurrence>) {
        self.fields.insert(name.into(), occurrences);
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }
}
