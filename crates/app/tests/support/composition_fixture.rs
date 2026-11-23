use any_cal_anytype_adapter::{
    AnytypeDiscoveryTransport, AnytypeTransport, AnytypeTypedTransport, DiscoveredMember,
    DiscoveredProperty, DiscoveredSpace, DiscoveredTag, DiscoveredType, DiscoveredView,
    FakeAnytypeTransport, ObjectRecord, Page, TransportError,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub struct CompositionSchemaFixture {
    pub types: Vec<DiscoveredType>,
    pub properties: Vec<DiscoveredProperty>,
}

impl CompositionSchemaFixture {
    pub fn new(type_keys: &[&str], properties: &[(&str, &str)]) -> Self {
        Self {
            types: type_keys
                .iter()
                .map(|key| DiscoveredType {
                    id: format!("type-{key}"),
                    key: (*key).into(),
                    name: format!("Fixture {key}"),
                    layout: None,
                    extra: BTreeMap::new(),
                })
                .collect(),
            properties: properties
                .iter()
                .map(|(key, format)| DiscoveredProperty {
                    id: format!("property-{key}"),
                    key: Some((*key).into()),
                    name: format!("Fixture {key}"),
                    format: (*format).into(),
                    extra: BTreeMap::new(),
                })
                .collect(),
        }
    }
}

/// The fake object store deliberately knows nothing about schema. Tests that
/// need discovery must declare each accessible Space's native schema here.
#[derive(Clone, Debug)]
pub struct CompositionFixtureTransport {
    pub inner: FakeAnytypeTransport,
    pub schemas: BTreeMap<String, CompositionSchemaFixture>,
    pub denied_read_spaces: BTreeSet<String>,
}

impl CompositionFixtureTransport {
    pub fn new(page_size: usize, schemas: BTreeMap<String, CompositionSchemaFixture>) -> Self {
        Self {
            inner: FakeAnytypeTransport::new(page_size),
            schemas,
            denied_read_spaces: BTreeSet::new(),
        }
    }

    fn read_preflight(&self, space_id: &str) -> Result<(), TransportError> {
        if self.denied_read_spaces.contains(space_id) {
            Err(TransportError::Auth)
        } else {
            Ok(())
        }
    }

    fn schema(&self, space_id: &str) -> Result<&CompositionSchemaFixture, TransportError> {
        self.read_preflight(space_id)?;
        self.schemas.get(space_id).ok_or(TransportError::NotFound)
    }
}

impl AnytypeTransport for CompositionFixtureTransport {
    fn list_objects(
        &mut self,
        space_id: &str,
        cursor: Option<&str>,
    ) -> Result<Page<ObjectRecord>, TransportError> {
        self.read_preflight(space_id)?;
        self.inner.list_objects(space_id, cursor)
    }

    fn get_object(
        &mut self,
        space_id: &str,
        object_id: &str,
    ) -> Result<ObjectRecord, TransportError> {
        self.read_preflight(space_id)?;
        self.inner.get_object(space_id, object_id)
    }

    fn create_object(&mut self, object: ObjectRecord) -> Result<ObjectRecord, TransportError> {
        self.inner.create_object(object)
    }

    fn update_object(&mut self, object: ObjectRecord) -> Result<ObjectRecord, TransportError> {
        self.inner.update_object(object)
    }

    fn archive_object(
        &mut self,
        space_id: &str,
        object_id: &str,
    ) -> Result<ObjectRecord, TransportError> {
        self.inner.archive_object(space_id, object_id)
    }

    fn delete_object(
        &mut self,
        space_id: &str,
        object_id: &str,
    ) -> Result<ObjectRecord, TransportError> {
        self.inner.delete_object(space_id, object_id)
    }
}

impl AnytypeTypedTransport for CompositionFixtureTransport {
    fn create_object_with_type(
        &mut self,
        object: ObjectRecord,
        type_key: &str,
    ) -> Result<ObjectRecord, TransportError> {
        self.inner.create_object_with_type(object, type_key)
    }
}

impl AnytypeDiscoveryTransport for CompositionFixtureTransport {
    fn list_spaces(
        &mut self,
        offset: Option<&str>,
    ) -> Result<Page<DiscoveredSpace>, TransportError> {
        assert!(offset.is_none());
        Ok(Page {
            data: self
                .schemas
                .keys()
                .filter(|space_id| !self.denied_read_spaces.contains(*space_id))
                .map(|space_id| DiscoveredSpace {
                    id: space_id.clone(),
                    name: format!("Fixture {space_id}"),
                    extra: BTreeMap::new(),
                })
                .collect(),
            next_offset: None,
        })
    }

    fn list_types(
        &mut self,
        space_id: &str,
        offset: Option<&str>,
    ) -> Result<Page<DiscoveredType>, TransportError> {
        assert!(offset.is_none());
        Ok(Page {
            data: self.schema(space_id)?.types.clone(),
            next_offset: None,
        })
    }

    fn list_properties(
        &mut self,
        space_id: &str,
        offset: Option<&str>,
    ) -> Result<Page<DiscoveredProperty>, TransportError> {
        assert!(offset.is_none());
        Ok(Page {
            data: self.schema(space_id)?.properties.clone(),
            next_offset: None,
        })
    }

    fn list_members(
        &mut self,
        space_id: &str,
        offset: Option<&str>,
    ) -> Result<Page<DiscoveredMember>, TransportError> {
        assert!(offset.is_none());
        self.schema(space_id)?;
        Ok(Page {
            data: Vec::new(),
            next_offset: None,
        })
    }

    fn list_tags(
        &mut self,
        _space_id: &str,
        _property_id: &str,
        _offset: Option<&str>,
    ) -> Result<Page<DiscoveredTag>, TransportError> {
        panic!("composition fixtures do not declare tag properties")
    }

    fn list_views(
        &mut self,
        _space_id: &str,
        _list_id: &str,
        _offset: Option<&str>,
    ) -> Result<Page<DiscoveredView>, TransportError> {
        panic!("composition fixtures do not declare list views")
    }
}
