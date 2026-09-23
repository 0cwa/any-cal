use crate::{binding_collection_ids, ensure_collection, AppConfig, ConfigError};
use any_cal_anytype_adapter::{AnytypeRepository, AnytypeTransport, Page, TransportError};
use any_cal_core::{DavRoute, DomainBinding, DomainBindings, DomainCollection};
use any_cal_dav_server::DavServer;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};

/// Cloneable transport handle shared by independently scoped repository
/// contexts. The mutex is the transport serialization boundary; repository
/// bindings, caches, and DAV collection metadata remain context-local.
pub(crate) struct SharedAnytypeTransport<T> {
    inner: Arc<Mutex<T>>,
}

impl<T> Clone for SharedAnytypeTransport<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T> SharedAnytypeTransport<T> {
    fn new(transport: T) -> Self {
        Self {
            inner: Arc::new(Mutex::new(transport)),
        }
    }

    fn lock(&self) -> Result<MutexGuard<'_, T>, TransportError> {
        self.inner.lock().map_err(|_| TransportError::Unavailable)
    }

    pub(crate) fn with_mut<R>(&self, operation: impl FnOnce(&mut T) -> R) -> R {
        let mut transport = self
            .inner
            .lock()
            .expect("shared Anytype transport mutex must not be poisoned");
        operation(&mut transport)
    }

    pub(crate) fn snapshot(&self) -> T
    where
        T: Clone,
    {
        self.inner
            .lock()
            .expect("shared Anytype transport mutex must not be poisoned")
            .clone()
    }
}

impl<T: AnytypeTransport> AnytypeTransport for SharedAnytypeTransport<T> {
    fn list_objects(
        &mut self,
        space_id: &str,
        cursor: Option<&str>,
    ) -> Result<Page<any_cal_anytype_adapter::ObjectRecord>, TransportError> {
        self.lock()?.list_objects(space_id, cursor)
    }

    fn get_object(
        &mut self,
        space_id: &str,
        object_id: &str,
    ) -> Result<any_cal_anytype_adapter::ObjectRecord, TransportError> {
        self.lock()?.get_object(space_id, object_id)
    }

    fn create_object(
        &mut self,
        object: any_cal_anytype_adapter::ObjectRecord,
    ) -> Result<any_cal_anytype_adapter::ObjectRecord, TransportError> {
        self.lock()?.create_object(object)
    }

    fn update_object(
        &mut self,
        object: any_cal_anytype_adapter::ObjectRecord,
    ) -> Result<any_cal_anytype_adapter::ObjectRecord, TransportError> {
        self.lock()?.update_object(object)
    }

    fn archive_object(
        &mut self,
        space_id: &str,
        object_id: &str,
    ) -> Result<any_cal_anytype_adapter::ObjectRecord, TransportError> {
        self.lock()?.archive_object(space_id, object_id)
    }

    fn delete_object(
        &mut self,
        space_id: &str,
        object_id: &str,
    ) -> Result<any_cal_anytype_adapter::ObjectRecord, TransportError> {
        self.lock()?.delete_object(space_id, object_id)
    }
}

pub(crate) struct DomainContext<T: AnytypeTransport> {
    pub(crate) binding: DomainBinding,
    pub(crate) server: DavServer<AnytypeRepository<SharedAnytypeTransport<T>>>,
}

pub(crate) struct DomainRepositoryRegistry<T: AnytypeTransport> {
    contexts: BTreeMap<String, DomainContext<T>>,
    primary_domain_id: String,
    transport: SharedAnytypeTransport<T>,
}

impl<T: AnytypeTransport> DomainRepositoryRegistry<T> {
    pub(crate) fn new(
        config: &AppConfig,
        bindings: &DomainBindings,
        transport: T,
        transport_mode: &str,
    ) -> Result<Self, ConfigError> {
        let primary_domain_id = bindings
            .bindings
            .first()
            .map(|binding| binding.domain_id.clone())
            .ok_or_else(|| ConfigError::Invalid("no domain bindings configured".into()))?;
        let transport = SharedAnytypeTransport::new(transport);
        let mut contexts = BTreeMap::new();

        for binding in &bindings.bindings {
            let (contacts, tasks) = binding_collection_ids(binding)?;
            let repository_binding = config.repository_binding_for(binding, transport_mode)?;
            let mut repository =
                AnytypeRepository::with_binding(transport.clone(), repository_binding.clone());
            ensure_collection(&mut repository.cache, contacts.clone(), "Contacts")?;
            ensure_collection(&mut repository.cache, tasks.clone(), "Tasks")?;
            contexts.insert(
                binding.domain_id.clone(),
                DomainContext {
                    binding: binding.clone(),
                    server: DavServer {
                        repository,
                        contacts,
                        tasks,
                    },
                },
            );
        }

        Ok(Self {
            contexts,
            primary_domain_id,
            transport,
        })
    }

    pub(crate) fn context(&self, domain_id: &str) -> Option<&DomainContext<T>> {
        self.contexts.get(domain_id)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.contexts.is_empty()
    }

    pub(crate) fn len(&self) -> usize {
        self.contexts.len()
    }

    pub(crate) fn has_collection_route(&self, collection: DomainCollection) -> bool {
        self.contexts
            .values()
            .flat_map(|context| context.binding.routes.iter())
            .any(|route| route.collection == collection)
    }

    pub(crate) fn resolve_route(&self, path: &str) -> Option<(&DomainContext<T>, &DavRoute)> {
        self.contexts.values().find_map(|context| {
            context.binding.routes.iter().find_map(|route| {
                let exact = path == route.path;
                let descendant = path
                    .strip_prefix(&route.path)
                    .is_some_and(|remainder| remainder.starts_with('/'));
                (exact || descendant).then_some((context, route))
            })
        })
    }

    pub(crate) fn space_ids(&self) -> BTreeSet<String> {
        self.contexts
            .values()
            .map(|context| context.binding.space_id.clone())
            .collect()
    }

    pub(crate) fn context_mut(&mut self, domain_id: &str) -> Option<&mut DomainContext<T>> {
        self.contexts.get_mut(domain_id)
    }

    pub(crate) fn primary_domain_id(&self) -> &str {
        &self.primary_domain_id
    }

    pub(crate) fn primary(&self) -> &DomainContext<T> {
        self.contexts
            .get(&self.primary_domain_id)
            .expect("primary domain context is constructed with registry")
    }

    pub(crate) fn primary_mut(&mut self) -> &mut DomainContext<T> {
        self.contexts
            .get_mut(&self.primary_domain_id)
            .expect("primary domain context is constructed with registry")
    }

    pub(crate) fn with_transport_mut<R>(&self, operation: impl FnOnce(&mut T) -> R) -> R {
        self.transport.with_mut(operation)
    }

    pub(crate) fn transport_snapshot(&self) -> T
    where
        T: Clone,
    {
        self.transport.snapshot()
    }
}
