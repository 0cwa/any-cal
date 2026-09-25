use crate::composition::CompositionSourceScope;
use crate::composition_orchestration::{CompositionOrchestrationError, MaterializationResult};
use crate::identity::CollectionKind;
use crate::AppGeneric;
use any_cal_anytype_adapter::{
    AnytypeTransport, AnytypeTypedTransport, ANYCAL_PERSON_CONTEXT_TYPE_KEY,
};
use any_cal_core::{project_person_context_source, CompositionProfile};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PersonContextMaterializationError {
    InvalidSourceScope,
    Orchestration(CompositionOrchestrationError),
}

impl<T: AnytypeTransport + AnytypeTypedTransport> AppGeneric<T> {
    /// Materialize private Person Contexts from canonical Contact resources.
    ///
    /// Only resource-qualified Contact scopes are accepted: broad collection
    /// composition would make accidental mass disclosure too easy for this
    /// user-facing proof. Authorization/privacy checks and destination routing
    /// remain delegated to the generic read-many/write-one orchestrator.
    pub fn materialize_person_contexts(
        &mut self,
        profile: &CompositionProfile,
        token: &str,
        sources: &[CompositionSourceScope],
    ) -> Result<Vec<MaterializationResult>, PersonContextMaterializationError> {
        if sources.is_empty()
            || sources.iter().any(|source| {
                source.collection != CollectionKind::Contacts
                    || source
                        .resource_id
                        .as_deref()
                        .is_none_or(|resource_id| resource_id.trim().is_empty())
            })
        {
            return Err(PersonContextMaterializationError::InvalidSourceScope);
        }

        self.materialize_references_with_type(
            profile,
            token,
            sources,
            ANYCAL_PERSON_CONTEXT_TYPE_KEY,
            |stored| {
                project_person_context_source(stored)
                    .expect("composition validates Contact kind before Person Context projection")
            },
        )
        .map_err(PersonContextMaterializationError::Orchestration)
    }
}

pub fn private_person_context_fields(
    notes: impl Into<String>,
    tags: impl IntoIterator<Item = String>,
) -> BTreeMap<String, serde_json::Value> {
    let notes = notes.into();
    let tags = tags.into_iter().collect::<Vec<_>>();
    let mut fields = BTreeMap::new();
    if !notes.trim().is_empty() {
        fields.insert("private_notes".into(), serde_json::json!(notes));
    }
    if !tags.is_empty() {
        fields.insert("private_tags".into(), serde_json::json!(tags));
    }
    fields
}
