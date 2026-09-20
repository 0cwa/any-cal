//! Component-aware iCalendar representation used for local protocol checks.
//!
//! The existing VTODO projection intentionally flattens properties into the
//! Anytype-facing envelope.  This module keeps the wire tree intact when a
//! caller needs to inspect or preserve nested components (for example
//! `VALARM` and `VTIMEZONE`) without making those components editable fields.

use crate::model::{parse_property_head, serialize_parameter_value, serialize_property_value};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Calendar {
    pub component: Component,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Component {
    pub name: String,
    pub entries: Vec<Entry>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Entry {
    Property(Property),
    Component(Component),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Property {
    pub name: String,
    pub params: BTreeMap<String, Vec<String>>,
    pub value: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "malformed iCalendar: {}", self.0)
    }
}

impl std::error::Error for ParseError {}

impl Calendar {
    pub fn parse(input: &str) -> Result<Self, ParseError> {
        let mut stack: Vec<Component> = Vec::new();
        let mut root: Option<Component> = None;
        for line in unfold(input) {
            if let Some(name) = line.strip_prefix("BEGIN:") {
                let name = component_name(name)?;
                stack.push(Component {
                    name,
                    entries: Vec::new(),
                });
                continue;
            }
            if let Some(name) = line.strip_prefix("END:") {
                let name = component_name(name)?;
                let component = stack
                    .pop()
                    .ok_or_else(|| ParseError(format!("END:{name} without BEGIN")))?;
                if component.name != name {
                    return Err(ParseError(format!(
                        "END:{name} closes BEGIN:{}",
                        component.name
                    )));
                }
                if let Some(parent) = stack.last_mut() {
                    parent.entries.push(Entry::Component(component));
                } else if component.name == "VCALENDAR" && root.is_none() {
                    root = Some(component);
                } else {
                    return Err(ParseError("multiple or non-VCALENDAR roots".into()));
                }
                continue;
            }
            let current = stack
                .last_mut()
                .ok_or_else(|| ParseError("property outside component".into()))?;
            let (left, value) = line
                .split_once(':')
                .ok_or_else(|| ParseError(format!("missing ':' in {line:?}")))?;
            let (name, params) = parse_property_head(left).map_err(ParseError)?;
            let params = params
                .into_iter()
                .map(|(key, values)| {
                    (
                        key,
                        values.into_iter().map(|value| unescape(&value)).collect(),
                    )
                })
                .collect();
            current.entries.push(Entry::Property(Property {
                name,
                params,
                value: unescape(value),
            }));
        }
        if !stack.is_empty() {
            return Err(ParseError("unbalanced component boundaries".into()));
        }
        let component = root.ok_or_else(|| ParseError("missing VCALENDAR root".into()))?;
        Ok(Self { component })
    }

    pub fn serialize(&self) -> String {
        let mut out = String::new();
        write_component(&mut out, &self.component);
        out
    }

    pub fn root(&self) -> &Component {
        &self.component
    }
}

impl Component {
    pub fn components_named(&self, name: &str) -> impl Iterator<Item = &Component> {
        let name = name.to_ascii_uppercase();
        self.entries.iter().filter_map(move |entry| match entry {
            Entry::Component(component) if component.name == name => Some(component),
            _ => None,
        })
    }

    pub fn properties_named(&self, name: &str) -> impl Iterator<Item = &Property> {
        let name = name.to_ascii_uppercase();
        self.entries.iter().filter_map(move |entry| match entry {
            Entry::Property(property) if property.name == name => Some(property),
            _ => None,
        })
    }
}

fn component_name(value: &str) -> Result<String, ParseError> {
    if value.is_empty() || value.contains(':') || value.contains(';') {
        return Err(ParseError("invalid component name".into()));
    }
    Ok(value.to_ascii_uppercase())
}

fn write_component(out: &mut String, component: &Component) {
    out.push_str("BEGIN:");
    out.push_str(&component.name);
    out.push_str("\r\n");
    for entry in &component.entries {
        match entry {
            Entry::Property(property) => {
                out.push_str(&property.name);
                for (key, values) in &property.params {
                    out.push(';');
                    out.push_str(key);
                    out.push('=');
                    out.push_str(
                        &values
                            .iter()
                            .map(|value| serialize_parameter_value(value))
                            .collect::<Vec<_>>()
                            .join(","),
                    );
                }
                out.push(':');
                out.push_str(&serialize_property_value(&property.name, &property.value));
                out.push_str("\r\n");
            }
            Entry::Component(child) => write_component(out, child),
        }
    }
    out.push_str("END:");
    out.push_str(&component.name);
    out.push_str("\r\n");
}

fn unfold(input: &str) -> Vec<String> {
    let mut result = Vec::new();
    for raw in input.replace("\r\n", "\n").replace('\r', "\n").split('\n') {
        if raw.is_empty() {
            continue;
        }
        if raw.starts_with(' ') || raw.starts_with('\t') {
            let Some(last) = result.last_mut() else {
                // A continuation without a preceding content line is not a
                // property and is rejected by the parser as such.
                result.push(raw.to_owned());
                continue;
            };
            last.push_str(&raw[1..]);
        } else {
            result.push(raw.to_owned());
        }
    }
    result
}

fn unescape(value: &str) -> String {
    let mut output = String::new();
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        if character == '\\' {
            match chars.next() {
                Some('n') | Some('N') => output.push('\n'),
                Some(next) => output.push(next),
                None => output.push('\\'),
            }
        } else {
            output.push(character);
        }
    }
    output
}
