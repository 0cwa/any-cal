use crate::{
    Collection, CollectionId, FailureMode, ModifiedAt, Repository, RepositoryError,
    ResourceEnvelope, ResourceId, StoredResource, WriteCondition,
};
use std::collections::{BTreeMap, HashSet, VecDeque};

#[derive(Clone, Debug)]
pub struct MemoryRepository {
    collections: BTreeMap<CollectionId, Collection>,
    resources: BTreeMap<ResourceId, StoredResource>,
    failures: VecDeque<FailureMode>,
    clock: ModifiedAt,
}

impl MemoryRepository {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the synthetic wall clock used by subsequent writes.  This is
    /// intentionally explicit so date-precondition tests never depend on the
    /// host clock.
    pub fn set_clock_seconds(&mut self, seconds: u64) {
        self.clock = ModifiedAt::from_unix_seconds(seconds);
    }

    /// Queue a one-shot failure. Failures are consumed in FIFO order by the
    /// operation for which they are relevant; this keeps tests deterministic.
    pub fn inject_failure(&mut self, failure: FailureMode) {
        self.failures.push_back(failure);
    }

    fn take_failure(&mut self, expected: FailureMode) -> bool {
        if self.failures.front() == Some(&expected) {
            self.failures.pop_front();
            true
        } else {
            false
        }
    }

    fn check_read_failure(&mut self) -> Result<(), RepositoryError> {
        if self.take_failure(FailureMode::Timeout) {
            return Err(RepositoryError::Timeout);
        }
        if self.take_failure(FailureMode::MalformedState) {
            return Err(RepositoryError::MalformedState);
        }
        if self.take_failure(FailureMode::ReadAfterWriteDelay) {
            return Err(RepositoryError::ReadAfterWriteDelay);
        }
        Ok(())
    }

    fn check_write_failure(&mut self) -> Result<(), RepositoryError> {
        if self.take_failure(FailureMode::Timeout) {
            return Err(RepositoryError::Timeout);
        }
        if self.take_failure(FailureMode::MalformedState) {
            return Err(RepositoryError::MalformedState);
        }
        Ok(())
    }

    fn etag(envelope: &ResourceEnvelope) -> Result<crate::ETag, RepositoryError> {
        let json = envelope
            .canonical_json()
            .map_err(|error| RepositoryError::InvalidEnvelope(error.to_string()))?;
        Ok(crate::typed_etag_for_bytes(json.as_bytes()))
    }

    fn check_condition(
        condition: &WriteCondition,
        current: Option<&StoredResource>,
    ) -> Result<(), RepositoryError> {
        let actual = current.map(|resource| resource.etag.clone());
        match condition {
            WriteCondition::Unconditional => Ok(()),
            WriteCondition::IfNoneMatch if current.is_none() => Ok(()),
            WriteCondition::IfNoneMatch => Err(RepositoryError::PreconditionFailed {
                expected: None,
                actual,
            }),
            WriteCondition::IfMatch(expected)
                if current.is_some_and(|resource| resource.etag == *expected) =>
            {
                Ok(())
            }
            WriteCondition::IfMatch(expected) => Err(RepositoryError::PreconditionFailed {
                expected: Some(expected.clone()),
                actual,
            }),
        }
    }

    fn validate_envelope(&self, envelope: &ResourceEnvelope) -> Result<(), RepositoryError> {
        envelope
            .validate()
            .map_err(|error| RepositoryError::InvalidEnvelope(error.to_string()))?;
        if !self.collections.contains_key(&envelope.collection_id) {
            return Err(RepositoryError::CollectionNotFound(
                envelope.collection_id.clone(),
            ));
        }
        Ok(())
    }

    fn identities_in_use(
        &self,
        envelope: &ResourceEnvelope,
        except: Option<&ResourceId>,
    ) -> Option<String> {
        self.resources.values().find_map(|resource| {
            if except == Some(&resource.envelope.resource_id) {
                return None;
            }
            if resource.envelope.anytype_object_id == envelope.anytype_object_id {
                Some(format!("anytype:{}", envelope.anytype_object_id))
            } else if resource.envelope.dav_uid == envelope.dav_uid {
                Some(format!("dav_uid:{}", envelope.dav_uid))
            } else {
                None
            }
        })
    }

    pub fn rebuild_indexes(&self) -> (HashSet<String>, HashSet<String>) {
        let anytype_ids = self
            .resources
            .values()
            .map(|resource| resource.envelope.anytype_object_id.to_string())
            .collect();
        let dav_uids = self
            .resources
            .values()
            .map(|resource| resource.envelope.dav_uid.to_string())
            .collect();
        (anytype_ids, dav_uids)
    }
}

impl Default for MemoryRepository {
    fn default() -> Self {
        Self {
            collections: BTreeMap::new(),
            resources: BTreeMap::new(),
            failures: VecDeque::new(),
            clock: ModifiedAt::now(),
        }
    }
}

impl Repository for MemoryRepository {
    fn list_collections(&mut self) -> Result<Vec<Collection>, RepositoryError> {
        self.check_read_failure()?;
        Ok(self.collections.values().cloned().collect())
    }

    fn get_collection(&mut self, id: &CollectionId) -> Result<Option<Collection>, RepositoryError> {
        self.check_read_failure()?;
        Ok(self.collections.get(id).cloned())
    }

    fn create_collection(&mut self, collection: Collection) -> Result<(), RepositoryError> {
        self.check_write_failure()?;
        if self.take_failure(FailureMode::DuplicateWrite)
            || self.collections.contains_key(&collection.id)
        {
            return Err(RepositoryError::CollectionAlreadyExists(collection.id));
        }
        self.collections.insert(collection.id.clone(), collection);
        Ok(())
    }

    fn delete_collection(&mut self, id: &CollectionId) -> Result<(), RepositoryError> {
        self.check_write_failure()?;
        if self.collections.remove(id).is_none() {
            return Err(RepositoryError::CollectionNotFound(id.clone()));
        }
        self.resources
            .retain(|_, resource| &resource.envelope.collection_id != id);
        Ok(())
    }

    fn list_resources(
        &mut self,
        collection_id: &CollectionId,
        include_archived: bool,
    ) -> Result<Vec<StoredResource>, RepositoryError> {
        self.check_read_failure()?;
        if !self.collections.contains_key(collection_id) {
            return Err(RepositoryError::CollectionNotFound(collection_id.clone()));
        }
        Ok(self
            .resources
            .values()
            .filter(|resource| {
                &resource.envelope.collection_id == collection_id
                    && (include_archived || !resource.archived)
            })
            .cloned()
            .collect())
    }

    fn get_resource(&mut self, id: &ResourceId) -> Result<Option<StoredResource>, RepositoryError> {
        self.check_read_failure()?;
        Ok(self.resources.get(id).cloned())
    }

    fn create_resource(
        &mut self,
        envelope: ResourceEnvelope,
        condition: WriteCondition,
    ) -> Result<StoredResource, RepositoryError> {
        self.check_write_failure()?;
        self.validate_envelope(&envelope)?;
        let current = self.resources.get(&envelope.resource_id);
        Self::check_condition(&condition, current)?;
        if current.is_some() || self.take_failure(FailureMode::DuplicateWrite) {
            return Err(RepositoryError::ResourceAlreadyExists(envelope.resource_id));
        }
        if let Some(identity) = self.identities_in_use(&envelope, None) {
            return Err(RepositoryError::IdentityAlreadyExists(identity));
        }
        let stored = StoredResource {
            etag: Self::etag(&envelope)?,
            envelope,
            archived: false,
            modified_at: self.clock,
        };
        self.clock = self.clock.after(self.clock);
        self.resources
            .insert(stored.envelope.resource_id.clone(), stored.clone());
        Ok(stored)
    }

    fn update_resource(
        &mut self,
        mut envelope: ResourceEnvelope,
        condition: WriteCondition,
    ) -> Result<StoredResource, RepositoryError> {
        self.check_write_failure()?;
        self.validate_envelope(&envelope)?;
        let current = self
            .resources
            .get(&envelope.resource_id)
            .ok_or_else(|| RepositoryError::ResourceNotFound(envelope.resource_id.clone()))?;
        Self::check_condition(&condition, Some(current))?;
        if let Some(identity) = self.identities_in_use(&envelope, Some(&envelope.resource_id)) {
            return Err(RepositoryError::IdentityAlreadyExists(identity));
        }
        envelope.revision = current.envelope.revision.saturating_add(1);
        let archived = current.archived;
        let stored = StoredResource {
            etag: Self::etag(&envelope)?,
            envelope,
            archived,
            modified_at: current.modified_at.after(self.clock),
        };
        self.clock = stored.modified_at;
        self.resources
            .insert(stored.envelope.resource_id.clone(), stored.clone());
        Ok(stored)
    }

    fn archive_resource(
        &mut self,
        id: &ResourceId,
        condition: WriteCondition,
    ) -> Result<StoredResource, RepositoryError> {
        self.check_write_failure()?;
        if self.take_failure(FailureMode::ArchiveFailure) {
            return Err(RepositoryError::ArchiveFailure);
        }
        let current = self
            .resources
            .get(id)
            .ok_or_else(|| RepositoryError::ResourceNotFound(id.clone()))?;
        Self::check_condition(&condition, Some(current))?;
        let mut archived = current.clone();
        archived.archived = true;
        archived.modified_at = archived.modified_at.after(self.clock);
        self.clock = archived.modified_at;
        self.resources.insert(id.clone(), archived.clone());
        Ok(archived)
    }

    fn delete_resource(
        &mut self,
        id: &ResourceId,
        condition: WriteCondition,
    ) -> Result<StoredResource, RepositoryError> {
        self.check_write_failure()?;
        let current = self
            .resources
            .get(id)
            .ok_or_else(|| RepositoryError::ResourceNotFound(id.clone()))?;
        Self::check_condition(&condition, Some(current))?;
        Ok(self.resources.remove(id).expect("resource checked above"))
    }
}
