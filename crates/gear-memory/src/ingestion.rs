//! SourceRef ingestion: conversion from upstream documents to persistent store.
//!
//! This module provides high-level APIs for ingesting source documents into the store.
//! It handles the conversion from various document formats to the canonical SourceRef
//! contract and manages persistence via the Store trait.

use crate::{
    CustodyMutation, EventLogEntry, ProvenanceOperation, ProvenanceRecord, SafeMetadata, SourceRef,
    SourceState, Store, StoreError,
};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// A builder for creating SourceRef documents for ingestion.
/// Ensures all required fields are set before persistence.
#[derive(Debug, Clone)]
pub struct SourceRefBuilder {
    source_id: Option<String>,
    source_type: Option<crate::SourceType>,
    origin_product: Option<String>,
    uri: Option<String>,
    content_hash: Option<String>,
    provenance_id: Option<String>,
    canonical_title: Option<String>,
    canonical_text: Option<String>,
    metadata: SafeMetadata,
}

impl SourceRefBuilder {
    /// Create a new SourceRefBuilder.
    pub fn new() -> Self {
        Self {
            source_id: None,
            source_type: None,
            origin_product: None,
            uri: None,
            content_hash: None,
            provenance_id: None,
            canonical_title: None,
            canonical_text: None,
            metadata: SafeMetadata::default(),
        }
    }

    /// Set the source_id.
    pub fn source_id(mut self, id: impl Into<String>) -> Self {
        self.source_id = Some(id.into());
        self
    }

    /// Set the source type.
    pub fn source_type(mut self, t: crate::SourceType) -> Self {
        self.source_type = Some(t);
        self
    }

    /// Set the origin product.
    pub fn origin_product(mut self, product: impl Into<String>) -> Self {
        self.origin_product = Some(product.into());
        self
    }

    /// Set the URI.
    pub fn uri(mut self, uri: impl Into<String>) -> Self {
        self.uri = Some(uri.into());
        self
    }

    /// Set the content hash (must be SHA256).
    pub fn content_hash(mut self, hash: impl Into<String>) -> Self {
        self.content_hash = Some(hash.into());
        self
    }

    /// Set the provenance ID.
    pub fn provenance_id(mut self, id: impl Into<String>) -> Self {
        self.provenance_id = Some(id.into());
        self
    }

    /// Set the canonical title.
    pub fn canonical_title(mut self, title: impl Into<String>) -> Self {
        self.canonical_title = Some(title.into());
        self
    }

    /// Set the canonical text.
    pub fn canonical_text(mut self, text: impl Into<String>) -> Self {
        self.canonical_text = Some(text.into());
        self
    }

    /// Set custom metadata.
    pub fn metadata(mut self, metadata: SafeMetadata) -> Self {
        self.metadata = metadata;
        self
    }

    /// Build the SourceRef, validating that all required fields are present.
    pub fn build(self) -> Result<SourceRef, String> {
        let source_id = self.source_id.ok_or("source_id is required")?;
        let source_type = self.source_type.ok_or("source_type is required")?;
        let origin_product = self.origin_product.ok_or("origin_product is required")?;
        let content_hash = self.content_hash.ok_or("content_hash is required")?;
        let provenance_id = self.provenance_id.ok_or("provenance_id is required")?;

        Ok(SourceRef {
            source_id,
            source_type,
            origin_product,
            uri: self.uri,
            content_hash,
            provenance_id,
            state: SourceState::Active,
            created_at: OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .map_err(|e| format!("timestamp formatting failed: {}", e))?,
            canonical_title: self.canonical_title,
            canonical_text: self.canonical_text,
            metadata: self.metadata,
        })
    }
}

impl Default for SourceRefBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Ingest a SourceRef into the store with optional provenance tracking.
/// Returns the ingested source or an error.
pub fn ingest_source_ref<S: Store + ?Sized>(
    store: &S,
    source: SourceRef,
    actor_ref: &str,
    tool_ref: Option<&str>,
) -> Result<SourceRef, StoreError> {
    source
        .validate()
        .map_err(|e| StoreError::InvalidOperation(e.to_string()))?;

    let provenance = ProvenanceRecord {
        provenance_id: format!("prov_ingested_{}", source.source_id),
        actor_ref: actor_ref.to_string(),
        operation: ProvenanceOperation::Imported,
        inputs: vec![],
        outputs: vec![source.source_id.clone()],
        tool_ref: tool_ref.map(|s| s.to_string()),
        timestamp: source.created_at.clone(),
        metadata: SafeMetadata::default(),
    };
    let event = EventLogEntry {
        event_id: format!("evt_ingested_{}", source.source_id),
        event_type: "source.imported".to_string(),
        actor_ref: actor_ref.to_string(),
        target_ref: source.source_id.clone(),
        provenance_id: provenance.provenance_id.clone(),
        metadata: SafeMetadata::default(),
        created_at: source.created_at.clone(),
    };
    let mutation = CustodyMutation::new(None, source.clone(), provenance, event);
    store.apply_custody_mutation(&mutation)?;

    Ok(source)
}

/// Report on ingestion of multiple sources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestionReport {
    pub ingested_count: u64,
    pub failed_count: u64,
    pub total_count: u64,
}

/// Ingest multiple SourceRef documents, returning a report.
/// Failures in individual ingestions do not stop the batch.
pub fn ingest_batch<S: Store + ?Sized>(
    store: &S,
    sources: Vec<SourceRef>,
    actor_ref: &str,
    tool_ref: Option<&str>,
) -> IngestionReport {
    let total_count = sources.len() as u64;
    let mut ingested_count = 0u64;
    let mut failed_count = 0u64;

    for source in sources {
        match ingest_source_ref(store, source, actor_ref, tool_ref) {
            Ok(_) => ingested_count += 1,
            Err(_) => failed_count += 1,
        }
    }

    IngestionReport {
        ingested_count,
        failed_count,
        total_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FileStore, SourceType};
    use tempfile::TempDir;

    fn hash() -> String {
        format!("sha256:{}", "a".repeat(64))
    }

    #[test]
    fn builder_requires_all_fields() {
        let builder = SourceRefBuilder::new()
            .source_id("src_01")
            .origin_product("test");

        let error = builder.build().expect_err("missing fields");
        assert!(error.contains("source_type") || error.contains("content_hash"));
    }

    #[test]
    fn builder_creates_valid_source_ref() {
        let source = SourceRefBuilder::new()
            .source_id("src_01")
            .source_type(SourceType::Document)
            .origin_product("test-ingester")
            .uri("file:///tmp/test.md")
            .content_hash(hash())
            .provenance_id("prov_01")
            .canonical_title("Test Document")
            .canonical_text("This is the content.")
            .build()
            .expect("builder succeeds");

        assert_eq!(source.source_id, "src_01");
        assert_eq!(source.canonical_title, Some("Test Document".to_string()));
        assert_eq!(source.state, SourceState::Active);
        source.validate().expect("built source is valid");
    }

    #[test]
    fn ingest_source_ref_persists_and_creates_provenance() {
        let temp_dir = TempDir::new().expect("create temp dir");
        let store = FileStore::new(temp_dir.path()).expect("create store");

        let source = SourceRefBuilder::new()
            .source_id("src_01")
            .source_type(SourceType::Document)
            .origin_product("test-ingester")
            .content_hash(hash())
            .provenance_id("prov_01")
            .build()
            .expect("build source");

        let ingested = ingest_source_ref(&store, source, "test-actor", Some("test-tool"))
            .expect("ingest succeeds");

        // Verify the source was persisted
        let retrieved = store
            .get_source_ref("src_01")
            .expect("get succeeds")
            .expect("source exists");
        assert_eq!(retrieved.source_id, ingested.source_id);

        // Verify provenance was created
        let provenances = store
            .list_all_provenance_records()
            .expect("list provenances succeeds");
        assert_eq!(provenances.len(), 1);
        assert_eq!(provenances[0].operation, ProvenanceOperation::Imported);
        assert!(
            store
                .get_event_log_entry("evt_ingested_src_01")
                .expect("event lookup succeeds")
                .is_some()
        );
    }

    #[test]
    fn batch_ingestion_reports_success_and_failure() {
        let temp_dir = TempDir::new().expect("create temp dir");
        let store = FileStore::new(temp_dir.path()).expect("create store");

        let valid_source = SourceRefBuilder::new()
            .source_id("src_01")
            .source_type(SourceType::Document)
            .origin_product("test")
            .content_hash(hash())
            .provenance_id("prov_01")
            .build()
            .expect("build valid source");

        // Invalid source (missing content_hash format will fail validation)
        let mut invalid_source = valid_source.clone();
        invalid_source.content_hash = "not-a-hash".to_string();

        let report = ingest_batch(
            &store,
            vec![valid_source, invalid_source],
            "test-actor",
            Some("batch-tool"),
        );

        assert_eq!(report.total_count, 2);
        assert_eq!(report.ingested_count, 1);
        assert_eq!(report.failed_count, 1);
    }
}
