//! Wire-only DTOs for the pinned Anytype HTTP API.
//!
//! These types deliberately do not double as the repository/cache model. The
//! API has optional metadata, typed property values, wrappers, and extension
//! fields that the internal DAV cache does not need to expose.
use crate::{ObjectRecord, TransportError};
use any_cal_core::ResourceEnvelope;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct WireProperty {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    /// The old experimental endpoint returned a nested `value` object. Keep
    /// this only as a decode compatibility path; current API writes use the
    /// flat PropertyLinkWithValue shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl WireProperty {
    fn key(&self) -> Option<&str> {
        self.key.as_deref().or(self.name.as_deref())
    }

    fn value(&self) -> Option<String> {
        if let Some(value) = &self.value {
            return value_to_string(value);
        }
        // PropertyWithValue/PropertyLinkWithValue are tagged by the typed
        // field itself. The order is deliberately explicit and mirrors the
        // API's PropertyFormat variants, rather than depending on JSON map
        // ordering.
        [
            "text",
            "number",
            "date",
            "url",
            "email",
            "phone",
            "checkbox",
            "select",
            "multi_select",
            "objects",
            "files",
        ]
        .iter()
        .find_map(|tag| self.extra.get(*tag).and_then(value_to_string))
    }

    fn format(&self) -> Option<String> {
        self.format.clone().or_else(|| {
            [
                "text",
                "number",
                "date",
                "url",
                "email",
                "phone",
                "checkbox",
                "select",
                "multi_select",
                "objects",
                "files",
            ]
            .iter()
            .find(|tag| self.extra.contains_key(**tag))
            .map(|tag| (*tag).to_owned())
        })
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct WireType {
    #[serde(default)]
    pub key: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct WireObject {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub space_id: Option<String>,
    /// Legacy response spelling. Current API responses expose `type: {key}`.
    #[serde(default)]
    pub type_key: Option<String>,
    #[serde(rename = "type", default)]
    pub object_type: Option<WireType>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub icon: Option<Value>,
    /// `body` is retained for old response fixtures. API 2025-11-08 returns
    /// the full object's markdown body as `markdown`.
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub markdown: Option<String>,
    #[serde(default, deserialize_with = "deserialize_properties")]
    pub properties: Vec<WireProperty>,
    #[serde(default)]
    pub archived: bool,
    /// Not present in the official Object/ObjectWithBody schemas.  It remains
    /// optional for compatibility with old fixtures and the fake transport.
    #[serde(default)]
    pub revision: Option<u64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct WireListResponse {
    #[serde(default)]
    pub data: Vec<WireObject>,
    #[serde(default, alias = "next_cursor")]
    pub next_offset: Option<Value>,
    #[serde(default)]
    pub pagination: Option<WirePagination>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct WirePagination {
    #[serde(default)]
    pub has_more: bool,
    #[serde(default)]
    pub offset: u64,
    #[serde(default)]
    pub limit: u64,
    #[serde(default)]
    pub total: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WireObjectResponse {
    pub object: WireObject,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl WireListResponse {
    pub fn next_offset_string(&self) -> Option<String> {
        self.next_offset
            .as_ref()
            .and_then(value_to_string)
            .or_else(|| {
                self.pagination.as_ref().and_then(|pagination| {
                    pagination.has_more.then(|| {
                        pagination
                            .offset
                            .saturating_add(self.data.len() as u64)
                            .to_string()
                    })
                })
            })
    }
}

pub fn decode_list(
    text: &str,
    fallback_space: Option<&str>,
) -> Result<(Vec<ObjectRecord>, Option<String>), TransportError> {
    let response: WireListResponse =
        serde_json::from_str(text).map_err(|_| TransportError::Malformed)?;
    let next_offset = response.next_offset_string();
    let objects = response
        .data
        .into_iter()
        .map(|object| object.into_record(fallback_space))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((objects, next_offset))
}

pub fn decode_object(
    text: &str,
    fallback_space: Option<&str>,
) -> Result<ObjectRecord, TransportError> {
    let wrapped = serde_json::from_str::<WireObjectResponse>(text)
        .map(|response| response.object)
        .or_else(|_| serde_json::from_str::<WireObject>(text))
        .map_err(|_| TransportError::Malformed)?;
    wrapped.into_record(fallback_space)
}

pub fn encode_create(object: &ObjectRecord) -> Result<String, TransportError> {
    let value = object_to_wire(object, Operation::Create)?;
    serde_json::to_string(&value).map_err(|_| TransportError::Malformed)
}

pub fn encode_update(object: &ObjectRecord) -> Result<String, TransportError> {
    let value = object_to_wire(object, Operation::Update)?;
    serde_json::to_string(&value).map_err(|_| TransportError::Malformed)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Operation {
    Create,
    Update,
}

fn object_to_wire(
    object: &ObjectRecord,
    operation: Operation,
) -> Result<Map<String, Value>, TransportError> {
    if object.id.is_empty() {
        return Err(TransportError::InvalidRequest(
            "object id must not be empty".into(),
        ));
    }

    let mut value = Map::new();
    let body_field = match operation {
        Operation::Create => "body",
        Operation::Update => "markdown",
    };
    value.insert(
        body_field.into(),
        Value::String(encode_markdown_body(&object.body)),
    );
    if let Some(name) = display_name(object) {
        value.insert("name".into(), Value::String(name));
    }
    if operation == Operation::Create {
        value.insert("type_key".into(), Value::String("page".into()));
    }
    value.insert(
        "properties".into(),
        Value::Array(
            object
                .properties
                .iter()
                .map(|(name, property)| {
                    let mut item = Map::new();
                    item.insert("key".into(), Value::String(name.clone()));
                    let (tag, typed) = typed_property_value(
                        name,
                        property,
                        object.property_formats.get(name).map(String::as_str),
                    )?;
                    item.insert(tag, typed);
                    Ok(Value::Object(item))
                })
                .collect::<Result<Vec<_>, TransportError>>()?,
        ),
    );
    Ok(value)
}

impl WireObject {
    pub fn into_record(self, fallback_space: Option<&str>) -> Result<ObjectRecord, TransportError> {
        let id = self.id.ok_or(TransportError::Malformed)?;
        let space_id = self
            .space_id
            .or_else(|| fallback_space.map(str::to_owned))
            .ok_or(TransportError::Malformed)?;
        let property_formats = self
            .properties
            .iter()
            .filter_map(|property| Some((property.key()?.to_owned(), property.format()?)))
            .collect::<BTreeMap<_, _>>();
        let properties = self
            .properties
            .into_iter()
            .map(|property| {
                let key = property.key().ok_or(TransportError::Malformed)?.to_owned();
                Ok((key, property.value().unwrap_or_default()))
            })
            .collect::<Result<Vec<_>, TransportError>>()?;
        Ok(ObjectRecord {
            id,
            space_id,
            properties,
            property_formats,
            body: decode_markdown_body(self.markdown.or(self.body).unwrap_or_default()),
            archived: self.archived,
            revision: self.revision.unwrap_or_default(),
        })
    }
}

/// Anytype stores object bodies as Markdown. Plain JSON is rewritten by the
/// Markdown normalizer (for example, `_` becomes `\_`), which corrupts the
/// canonical DAV envelope. A fenced code block is treated as literal text by
/// the service; hydration removes only that transport wrapper and keeps the
/// internal repository body as canonical JSON.
fn encode_markdown_body(body: &str) -> String {
    if is_markdown_fence(body) {
        body.to_owned()
    } else {
        format!("```json\n{body}\n```")
    }
}

fn decode_markdown_body(body: String) -> String {
    let trimmed = body.trim();
    if !trimmed.starts_with("```") || !trimmed.ends_with("```") {
        return body;
    }
    let mut inner = &trimmed[3..trimmed.len() - 3];
    if let Some(without_language) = inner.strip_prefix("json") {
        if without_language
            .chars()
            .next()
            .is_some_and(char::is_whitespace)
        {
            inner = without_language;
        }
    }
    inner.trim().to_owned()
}

fn is_markdown_fence(body: &str) -> bool {
    let trimmed = body.trim();
    trimmed.starts_with("```") && trimmed.ends_with("```")
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Null => None,
        Value::Object(object) => [
            "text", "checkbox", "number", "date", "url", "email", "phone",
        ]
        .iter()
        .find_map(|key| object.get(*key).and_then(value_to_string))
        .or_else(|| serde_json::to_string(value).ok()),
        other => serde_json::to_string(other).ok(),
    }
}

/// Anytype's property-link value is a tagged object. A bare JSON string is
/// rejected by the pinned CLI because it cannot determine the property type.
/// The internal cache has no schema handle, so use stable DAV/property-name
/// conventions for the scalar types and retain relation values as `objects`.
fn typed_property_value(
    name: &str,
    value: &str,
    format: Option<&str>,
) -> Result<(String, Value), TransportError> {
    let key = name.to_ascii_lowercase();
    let format = format.map(str::to_ascii_lowercase);
    let (tag, typed) = if format.as_deref() == Some("checkbox")
        || (format.is_none() && (key == "done" || key.ends_with(".done") || key == "checkbox"))
    {
        (
            "checkbox",
            value.parse::<bool>().map(Value::Bool).map_err(|_| {
                TransportError::InvalidRequest(format!(
                    "checkbox property {name} must be true or false"
                ))
            })?,
        )
    } else if format.as_deref() == Some("number")
        || (format.is_none() && (key == "number" || key.ends_with(".number")))
    {
        (
            "number",
            value
                .parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map(Value::Number)
                .ok_or_else(|| {
                    TransportError::InvalidRequest(format!("number property {name} is invalid"))
                })?,
        )
    } else if format.as_deref() == Some("date")
        || (format.is_none() && (key == "date" || key.ends_with(".date")))
    {
        ("date", Value::String(value.into()))
    } else if matches!(
        format.as_deref(),
        Some("multi_select") | Some("objects") | Some("files")
    ) || (format.is_none()
        && (key == "relation" || key.ends_with(".relation") || key == "objects"))
    {
        let values = json_link_array(value).unwrap_or_else(|| {
            value
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(|item| Value::String(item.into()))
                .collect()
        });
        let tag = format.as_deref().unwrap_or("objects");
        (tag, Value::Array(values))
    } else if format.as_deref() == Some("select") {
        (
            "select",
            select_link_value(value).unwrap_or_else(|| Value::String(value.into())),
        )
    } else {
        let tag = format.as_deref().unwrap_or("text");
        (tag, Value::String(value.into()))
    };
    Ok((tag.into(), typed))
}

fn json_link_array(value: &str) -> Option<Vec<Value>> {
    serde_json::from_str::<Value>(value)
        .ok()?
        .as_array()?
        .iter()
        .map(|item| match item {
            Value::String(item) => Some(Value::String(item.to_owned())),
            Value::Object(object) => object
                .get("key")
                .or_else(|| object.get("id"))
                .or_else(|| object.get("name"))
                .and_then(Value::as_str)
                .map(|item| Value::String(item.to_owned())),
            _ => None,
        })
        .collect()
}

fn select_link_value(value: &str) -> Option<Value> {
    let object = serde_json::from_str::<Value>(value).ok()?;
    let object = object.as_object()?;
    object.get("key").or_else(|| object.get("id")).cloned()
}

fn deserialize_properties<'de, D>(deserializer: D) -> Result<Vec<WireProperty>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let values = Vec::<Value>::deserialize(deserializer)?;
    values
        .into_iter()
        .map(|value| match value {
            Value::Object(_) => serde_json::from_value(value).map_err(serde::de::Error::custom),
            Value::Array(mut pair) if pair.len() == 2 => {
                let value = pair.pop().expect("length checked");
                let name = pair
                    .pop()
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .ok_or_else(|| serde::de::Error::custom("property name must be a string"))?;
                Ok(WireProperty {
                    key: Some(name),
                    value: Some(value),
                    ..WireProperty::default()
                })
            }
            _ => Err(serde::de::Error::custom("invalid property shape")),
        })
        .collect()
}

fn display_name(object: &ObjectRecord) -> Option<String> {
    if let Some(name) = object
        .properties
        .iter()
        .find_map(|(key, value)| (key.eq_ignore_ascii_case("name")).then_some(value))
    {
        if !name.trim().is_empty() {
            return Some(name.clone());
        }
    }

    let envelope = serde_json::from_str::<ResourceEnvelope>(&object.body).ok()?;
    let field = match envelope.kind {
        any_cal_core::DavKind::Contact => "FN",
        any_cal_core::DavKind::Task => "SUMMARY",
        any_cal_core::DavKind::ContactGroup | any_cal_core::DavKind::Event => "SUMMARY",
    };
    envelope
        .document
        .content
        .fields
        .get(field)
        .and_then(|values| values.first())
        .map(|occurrence| occurrence.value.clone())
        .filter(|name| !name.trim().is_empty())
}
