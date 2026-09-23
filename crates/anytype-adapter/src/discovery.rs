//! Read-only Anytype v1 discovery models.
//!
//! Discovery is deliberately separate from the CRUD transport: the CRUD
//! transport is the routing/mutation contract, while this module only
//! inventories capabilities and schema metadata. Callers must never use
//! discovery results as an alternate routing authority.

use crate::TransportError;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const DISCOVERY_PAGE_LIMIT: u64 = 100;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveryPage<T> {
    pub data: Vec<T>,
    pub next_offset: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TypeRecord {
    pub id: String,
    pub key: String,
    pub name: String,
    pub layout: String,
    #[serde(default)]
    pub icon: Option<Value>,
    #[serde(default)]
    pub object: Option<String>,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PropertyRecord {
    pub id: String,
    pub key: String,
    pub name: String,
    pub format: String,
    #[serde(default)]
    pub object: Option<String>,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TagRecord {
    pub id: String,
    pub key: String,
    pub name: String,
    pub color: String,
    #[serde(default)]
    pub object: Option<String>,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MemberRecord {
    #[serde(alias = "id")]
    pub profile_id: String,
    pub name: String,
    #[serde(default, alias = "identity")]
    pub network_id: Option<String>,
    #[serde(default)]
    pub global_name: Option<String>,
    pub status: String,
    pub role: String,
    #[serde(default)]
    pub icon: Option<Value>,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ViewRecord {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub layout: String,
    #[serde(default)]
    pub filters: Value,
    #[serde(default)]
    pub sorts: Value,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

pub trait AnytypeDiscovery {
    fn list_types(
        &mut self,
        space_id: &str,
        offset: Option<&str>,
    ) -> Result<DiscoveryPage<TypeRecord>, TransportError>;

    fn list_properties(
        &mut self,
        space_id: &str,
        offset: Option<&str>,
    ) -> Result<DiscoveryPage<PropertyRecord>, TransportError>;

    fn list_members(
        &mut self,
        space_id: &str,
        offset: Option<&str>,
    ) -> Result<DiscoveryPage<MemberRecord>, TransportError>;

    fn list_tags(
        &mut self,
        space_id: &str,
        property_id: &str,
        offset: Option<&str>,
    ) -> Result<DiscoveryPage<TagRecord>, TransportError>;

    fn list_views(
        &mut self,
        space_id: &str,
        list_id: &str,
        offset: Option<&str>,
    ) -> Result<DiscoveryPage<ViewRecord>, TransportError>;
}

#[derive(Clone, Debug, Default, Deserialize)]
struct WirePage<T> {
    #[serde(default)]
    data: Vec<T>,
    #[serde(default, alias = "next_cursor")]
    next_offset: Option<Value>,
    #[serde(default)]
    pagination: Option<WirePagination>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct WirePagination {
    #[serde(default)]
    has_more: bool,
    #[serde(default)]
    offset: u64,
    #[serde(default)]
    limit: u64,
}

pub(crate) fn decode_page<T: DeserializeOwned>(
    text: &str,
) -> Result<DiscoveryPage<T>, TransportError> {
    let page: WirePage<T> =
        serde_json::from_str(text).map_err(|_| TransportError::Malformed)?;
    let next_offset = page
        .next_offset
        .as_ref()
        .and_then(offset_string)
        .or_else(|| {
            page.pagination.as_ref().and_then(|pagination| {
                pagination
                    .has_more
                    .then(|| pagination.offset.saturating_add(pagination.limit).to_string())
            })
        });
    Ok(DiscoveryPage {
        data: page.data,
        next_offset,
    })
}

fn offset_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}
