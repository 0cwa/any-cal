use crate::model::{
    parse_property_head, serialize_parameter_value, serialize_property_value, Occurrence,
    StructuredDocument,
};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VTodo {
    pub fields: StructuredDocument,
    pub calendar_metadata: StructuredDocument,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VTodoError {
    Malformed(String),
}
impl fmt::Display for VTodoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "malformed VTODO: {}",
            match self {
                Self::Malformed(s) => s,
            }
        )
    }
}
impl std::error::Error for VTodoError {}

pub fn parse(input: &str) -> Result<VTodo, VTodoError> {
    let lines = unfold(input);
    if lines.len() < 3
        || lines.first().map(String::as_str) != Some("BEGIN:VCALENDAR")
        || lines.last().map(String::as_str) != Some("END:VCALENDAR")
    {
        return Err(VTodoError::Malformed("missing VCALENDAR wrapper".into()));
    }
    let start = lines
        .iter()
        .position(|x| x == "BEGIN:VTODO")
        .ok_or_else(|| VTodoError::Malformed("missing BEGIN:VTODO".into()))?;
    let end = lines
        .iter()
        .rposition(|x| x == "END:VTODO")
        .ok_or_else(|| VTodoError::Malformed("missing END:VTODO".into()))?;
    if end <= start {
        return Err(VTodoError::Malformed("invalid VTODO boundaries".into()));
    }
    let mut fields = BTreeMap::new();
    let mut calendar_metadata = BTreeMap::new();
    for line in &lines[1..start] {
        let (left, value) = line
            .split_once(':')
            .ok_or_else(|| VTodoError::Malformed(format!("missing ':' in {line:?}")))?;
        let (name, params) = parse_property_head(left).map_err(VTodoError::Malformed)?;
        let params = params
            .into_iter()
            .map(|(key, values)| {
                (
                    key,
                    values.into_iter().map(|value| unescape(&value)).collect(),
                )
            })
            .collect();
        calendar_metadata
            .entry(name)
            .or_insert_with(Vec::new)
            .push(Occurrence {
                value: unescape(value),
                params,
            });
    }
    for line in &lines[start + 1..end] {
        let (left, value) = line
            .split_once(':')
            .ok_or_else(|| VTodoError::Malformed(format!("missing ':' in {line:?}")))?;
        let (name, params) = parse_property_head(left).map_err(VTodoError::Malformed)?;
        let params = params
            .into_iter()
            .map(|(key, values)| {
                (
                    key,
                    values.into_iter().map(|value| unescape(&value)).collect(),
                )
            })
            .collect();
        fields
            .entry(name)
            .or_insert_with(Vec::new)
            .push(Occurrence {
                value: unescape(value),
                params,
            });
    }
    Ok(VTodo {
        fields: StructuredDocument { fields },
        calendar_metadata: StructuredDocument {
            fields: calendar_metadata,
        },
    })
}

pub const NAMED_FIELDS: &[&str] = &[
    "UID",
    "SUMMARY",
    "DESCRIPTION",
    "DTSTART",
    "DUE",
    "COMPLETED",
    "STATUS",
    "PRIORITY",
    "PERCENT-COMPLETE",
    "CATEGORIES",
    "RELATED-TO",
    "URL",
];

/// Stable Anytype adapter seam; fields outside this projection are opaque.
pub type VTodoProjection = BTreeMap<String, Vec<Occurrence>>;

impl VTodo {
    pub fn projection(&self) -> VTodoProjection {
        NAMED_FIELDS
            .iter()
            .filter_map(|name| {
                self.fields
                    .fields
                    .get(*name)
                    .map(|v| ((*name).into(), v.clone()))
            })
            .collect()
    }
    pub fn apply_projection(&mut self, projection: &VTodoProjection) {
        for name in NAMED_FIELDS {
            if let Some(values) = projection.get(*name) {
                self.set_values(name, values.clone());
            }
        }
    }
    pub fn values(&self, name: &str) -> &[Occurrence] {
        self.fields
            .fields
            .get(&name.to_ascii_uppercase())
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
    pub fn set_values(&mut self, name: &str, values: Vec<Occurrence>) {
        self.fields.fields.insert(name.to_ascii_uppercase(), values);
    }
}

pub fn serialize(todo: &VTodo) -> String {
    let mut out = String::from("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\n");
    // VERSION is normalized to 2.0; other VCALENDAR-level properties and
    // their parameters are retained for the adapter.
    // They are emitted before the VTODO component on the next serialization.
    if !todo.calendar_metadata.fields.is_empty() {
        let mut header = String::new();
        for (name, values) in &todo.calendar_metadata.fields {
            if name == "VERSION" {
                continue;
            }
            for occurrence in values {
                header.push_str(name);
                for (key, params) in &occurrence.params {
                    header.push(';');
                    header.push_str(key);
                    header.push('=');
                    header.push_str(
                        &params
                            .iter()
                            .map(|v| serialize_parameter_value(v))
                            .collect::<Vec<_>>()
                            .join(","),
                    );
                }
                header.push(':');
                header.push_str(&serialize_property_value(name, &occurrence.value));
                header.push_str("\r\n");
            }
        }
        out = format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{header}BEGIN:VTODO\r\n");
    }
    for (name, values) in &todo.fields.fields {
        for occurrence in values {
            out.push_str(name);
            for (key, params) in &occurrence.params {
                out.push(';');
                out.push_str(key);
                out.push('=');
                out.push_str(
                    &params
                        .iter()
                        .map(|v| serialize_parameter_value(v))
                        .collect::<Vec<_>>()
                        .join(","),
                );
            }
            out.push(':');
            out.push_str(&serialize_property_value(name, &occurrence.value));
            out.push_str("\r\n");
        }
    }
    out.push_str("END:VTODO\r\nEND:VCALENDAR\r\n");
    out
}
fn unfold(input: &str) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for raw in input.replace("\r\n", "\n").replace('\r', "\n").split('\n') {
        if raw.is_empty() {
            continue;
        }
        if let Some(last) = result.last_mut() {
            if raw.starts_with(' ') || raw.starts_with('\t') {
                last.push_str(&raw[1..]);
                continue;
            }
        }
        result.push(raw.to_string());
    }
    result
}
fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') | Some('N') => out.push('\n'),
                Some(x) => out.push(x),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}
