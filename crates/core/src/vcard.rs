use crate::model::{
    parse_property_head, serialize_parameter_value, serialize_property_value, Occurrence,
    StructuredDocument,
};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Contact {
    pub fields: StructuredDocument,
    /// Container metadata (currently VERSION), retained separately.
    pub metadata: StructuredDocument,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VCardError {
    Malformed(String),
}

impl fmt::Display for VCardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "malformed vCard: {}",
            match self {
                Self::Malformed(s) => s,
            }
        )
    }
}
impl std::error::Error for VCardError {}

pub fn parse(input: &str) -> Result<Contact, VCardError> {
    let lines = unfold(input);
    if lines.len() < 3
        || lines.first().map(String::as_str) != Some("BEGIN:VCARD")
        || lines.last().map(String::as_str) != Some("END:VCARD")
    {
        return Err(VCardError::Malformed(
            "missing BEGIN:VCARD/END:VCARD".into(),
        ));
    }
    let mut fields: BTreeMap<String, Vec<Occurrence>> = BTreeMap::new();
    let mut metadata: BTreeMap<String, Vec<Occurrence>> = BTreeMap::new();
    for line in &lines[1..lines.len() - 1] {
        let (left, value) = line
            .split_once(':')
            .ok_or_else(|| VCardError::Malformed(format!("missing ':' in {line:?}")))?;
        let (name, params) = parse_property_head(left).map_err(VCardError::Malformed)?;
        // VERSION is a container-level protocol marker, not contact data.
        if name == "VERSION" {
            metadata.entry(name).or_default().push(Occurrence {
                value: unescape(value),
                params: BTreeMap::new(),
            });
            continue;
        }
        // `parse_property_head` already decodes quoted parameter values.
        // Do not apply vCard value escaping here: that would silently remove
        // literal backslashes (for example, an X-* label of `a\\b`).
        fields.entry(name).or_default().push(Occurrence {
            value: unescape(value),
            params,
        });
    }
    Ok(Contact {
        fields: StructuredDocument { fields },
        metadata: StructuredDocument { fields: metadata },
    })
}

pub const NAMED_FIELDS: &[&str] = &[
    "UID",
    "FN",
    "N",
    "TEL",
    "EMAIL",
    "ADR",
    "ORG",
    "TITLE",
    "CATEGORIES",
    "URL",
    "NOTE",
];

/// Stable seam consumed by the Anytype adapter: only named fields are
/// editable properties; all other occurrences remain in the DAV envelope.
pub type ContactProjection = BTreeMap<String, Vec<Occurrence>>;

impl Contact {
    pub fn projection(&self) -> ContactProjection {
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
    pub fn apply_projection(&mut self, projection: &ContactProjection) {
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

pub fn serialize(contact: &Contact) -> String {
    let mut out = String::from("BEGIN:VCARD\r\nVERSION:4.0\r\n");
    for (name, values) in &contact.fields.fields {
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
    out.push_str("END:VCARD\r\n");
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
