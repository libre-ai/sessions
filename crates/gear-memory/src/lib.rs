//! gear-memory — local-first source, memory, code graph, and provenance substrate.
//!
//! This crate deliberately stores and validates trustworthy references. It does
//! not decide what agents or products should do next.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

pub mod ingestion;

pub use ingestion::{IngestionReport, SourceRefBuilder, ingest_batch, ingest_source_ref};

/// Static project metadata used by the CLI and smoke tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectCard {
    pub name: &'static str,
    pub role: &'static str,
    pub responsibility: &'static str,
}

/// The repository's current scope card.
pub const PROJECT: ProjectCard = ProjectCard {
    name: "gear-memory",
    role: "local-first memory/source/code graph substrate",
    responsibility: "store, index, link, retrieve, and prove references; never decide",
};

/// Human-readable summary for CLI smoke runs.
pub fn summary() -> String {
    format!(
        "{} — {} ({})",
        PROJECT.name, PROJECT.role, PROJECT.responsibility
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    File,
    Url,
    FeedItem,
    NoteBlock,
    Transcript,
    Document,
    Dataset,
    Artifact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceState {
    Active,
    Stale,
    Deleted,
    Anonymized,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRef {
    pub source_id: String,
    pub source_type: SourceType,
    pub origin_product: String,
    pub uri: Option<String>,
    pub content_hash: String,
    pub provenance_id: String,
    pub state: SourceState,
    pub created_at: String,
    #[serde(default)]
    pub canonical_title: Option<String>,
    #[serde(default)]
    pub canonical_text: Option<String>,
    #[serde(default)]
    pub metadata: SafeMetadata,
}

impl SourceRef {
    pub fn validate(&self) -> Result<(), ContractValidationError> {
        validate_storage_id("source_id", &self.source_id)?;
        validate_non_empty_field("origin_product", &self.origin_product)?;
        validate_non_empty_field("provenance_id", &self.provenance_id)?;
        validate_sha256_field("content_hash", &self.content_hash)?;
        validate_timestamp_field("created_at", &self.created_at)?;
        validate_metadata(&self.metadata)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceOperation {
    Created,
    Imported,
    Transformed,
    Indexed,
    Linked,
    StaleMarked,
    Exported,
    Signed,
    Distributed,
    Revoked,
    Deleted,
    Anonymized,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvenanceRecord {
    pub provenance_id: String,
    pub actor_ref: String,
    pub operation: ProvenanceOperation,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub tool_ref: Option<String>,
    pub timestamp: String,
    pub metadata: SafeMetadata,
}

impl ProvenanceRecord {
    pub fn validate(&self) -> Result<(), ContractValidationError> {
        validate_storage_id("provenance_id", &self.provenance_id)?;
        validate_non_empty_field("actor_ref", &self.actor_ref)?;
        validate_non_empty_list("outputs", &self.outputs)?;
        validate_timestamp_field("timestamp", &self.timestamp)?;
        validate_metadata(&self.metadata)
    }

    pub fn stable_hash(&self) -> String {
        stable_json_hash(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexState {
    Pending,
    Indexed,
    Stale,
    Deleted,
    Anonymized,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexMetadata {
    pub schema_version: String,
    pub chunk_count: u32,
    pub embedding_model_ref: Option<String>,
    pub indexed_at: Option<String>,
}

impl IndexMetadata {
    pub fn validate(&self) -> Result<(), ContractValidationError> {
        if self.schema_version != "memory-entry.v0.1" {
            return Err(ContractValidationError::InvalidSchemaVersion {
                field: "index_metadata.schema_version",
                value: self.schema_version.clone(),
            });
        }

        if let Some(indexed_at) = &self.indexed_at {
            validate_timestamp_field("index_metadata.indexed_at", indexed_at)?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub memory_entry_id: String,
    pub source_ref: String,
    pub content_hash: String,
    pub index_state: IndexState,
    pub index_metadata: IndexMetadata,
    pub created_at: String,
}

impl MemoryEntry {
    pub fn validate(&self) -> Result<(), ContractValidationError> {
        validate_storage_id("memory_entry_id", &self.memory_entry_id)?;
        validate_non_empty_field("source_ref", &self.source_ref)?;
        validate_sha256_field("content_hash", &self.content_hash)?;
        self.index_metadata.validate()?;
        validate_timestamp_field("created_at", &self.created_at)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventLogEntry {
    pub event_id: String,
    pub event_type: String,
    pub actor_ref: String,
    pub target_ref: String,
    pub provenance_id: String,
    pub metadata: SafeMetadata,
    pub created_at: String,
}

impl EventLogEntry {
    pub fn validate(&self) -> Result<(), ContractValidationError> {
        validate_storage_id("event_id", &self.event_id)?;
        validate_non_empty_field("event_type", &self.event_type)?;
        validate_non_empty_field("actor_ref", &self.actor_ref)?;
        validate_non_empty_field("target_ref", &self.target_ref)?;
        validate_non_empty_field("provenance_id", &self.provenance_id)?;
        validate_timestamp_field("created_at", &self.created_at)?;
        validate_metadata(&self.metadata)
    }
}

/// One crash-consistent custody change. The target source state and its audit
/// records are applied as one logical mutation by every Store backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustodyMutation {
    pub mutation_id: String,
    pub expected_state: Option<SourceState>,
    pub source: SourceRef,
    pub provenance: ProvenanceRecord,
    pub event: EventLogEntry,
}

impl CustodyMutation {
    pub fn new(
        expected_state: Option<SourceState>,
        source: SourceRef,
        provenance: ProvenanceRecord,
        event: EventLogEntry,
    ) -> Self {
        Self {
            mutation_id: Self::derived_id(&source, &provenance, &event),
            expected_state,
            source,
            provenance,
            event,
        }
    }

    pub fn validate(&self) -> Result<(), ContractValidationError> {
        validate_storage_id("mutation_id", &self.mutation_id)?;
        self.source.validate()?;
        self.provenance.validate()?;
        self.event.validate()?;
        if !self
            .provenance
            .outputs
            .iter()
            .any(|output| output == &self.source.source_id)
        {
            return Err(ContractValidationError::InvalidReference(
                "mutation.provenance.outputs",
            ));
        }
        if self.event.target_ref != self.source.source_id {
            return Err(ContractValidationError::InvalidReference(
                "mutation.event.target_ref",
            ));
        }
        if self.event.provenance_id != self.provenance.provenance_id {
            return Err(ContractValidationError::InvalidReference(
                "mutation.event.provenance_id",
            ));
        }
        if self.mutation_id != Self::derived_id(&self.source, &self.provenance, &self.event) {
            return Err(ContractValidationError::InvalidReference(
                "mutation.mutation_id",
            ));
        }
        Ok(())
    }

    pub fn records_hash(&self) -> String {
        stable_json_hash(&(&self.source, &self.provenance, &self.event))
    }

    fn derived_id(
        source: &SourceRef,
        provenance: &ProvenanceRecord,
        event: &EventLogEntry,
    ) -> String {
        let mutation_hash = stable_json_hash(&(source, provenance, event));
        format!(
            "custody_{}",
            mutation_hash
                .strip_prefix("sha256:")
                .unwrap_or(&mutation_hash)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationOutcome {
    Applied,
    AlreadyApplied,
    Recovered,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutationReceipt {
    pub mutation_id: String,
    pub outcome: MutationOutcome,
    pub records_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeMap {
    pub code_map_id: String,
    pub root_source_ref: String,
    pub scope: CodeMapScope,
    pub parser_refs: Vec<String>,
    pub symbols: Vec<CodeSymbol>,
    pub edges: Vec<CodeEdge>,
    pub state: CodeMapState,
    pub created_at: String,
}

impl CodeMap {
    pub fn validate(&self) -> Result<(), ContractValidationError> {
        validate_storage_id("code_map_id", &self.code_map_id)?;
        validate_non_empty_field("root_source_ref", &self.root_source_ref)?;
        self.scope.validate()?;
        validate_non_empty_list("parser_refs", &self.parser_refs)?;
        for parser_ref in &self.parser_refs {
            validate_non_empty_field("parser_refs[]", parser_ref)?;
        }
        for symbol in &self.symbols {
            symbol.validate()?;
        }
        for edge in &self.edges {
            edge.validate()?;
        }
        validate_timestamp_field("created_at", &self.created_at)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeMapScope {
    pub repo_ref: Option<String>,
    pub revision: String,
    pub paths: Vec<String>,
}

impl CodeMapScope {
    pub fn validate(&self) -> Result<(), ContractValidationError> {
        validate_non_empty_field("scope.revision", &self.revision)?;
        validate_non_empty_list("scope.paths", &self.paths)?;
        for path in &self.paths {
            validate_non_empty_field("scope.paths[]", path)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeMapState {
    Active,
    Stale,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeSymbol {
    pub symbol_id: String,
    pub kind: CodeSymbolKind,
    pub name: String,
    pub source_ref: String,
    pub range: SourceRange,
    pub content_hash: String,
}

impl CodeSymbol {
    pub fn validate(&self) -> Result<(), ContractValidationError> {
        validate_non_empty_field("symbol_id", &self.symbol_id)?;
        validate_non_empty_field("name", &self.name)?;
        validate_non_empty_field("source_ref", &self.source_ref)?;
        self.range.validate()?;
        validate_sha256_field("content_hash", &self.content_hash)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeSymbolKind {
    Function,
    Type,
    Module,
    Trait,
    Interface,
    Route,
    Table,
    Test,
    Config,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRange {
    pub start_line: u32,
    pub end_line: u32,
}

impl SourceRange {
    pub fn validate(&self) -> Result<(), ContractValidationError> {
        if self.start_line == 0 {
            return Err(ContractValidationError::InvalidRange("start_line"));
        }
        if self.end_line == 0 || self.end_line < self.start_line {
            return Err(ContractValidationError::InvalidRange("end_line"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeEdge {
    pub from: String,
    pub to: String,
    pub kind: CodeEdgeKind,
}

impl CodeEdge {
    pub fn validate(&self) -> Result<(), ContractValidationError> {
        validate_non_empty_field("edge.from", &self.from)?;
        validate_non_empty_field("edge.to", &self.to)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeEdgeKind {
    Defines,
    Calls,
    Imports,
    Tests,
    Configures,
    Documents,
    GeneratedFrom,
    BelongsTo,
    Cites,
    DerivedFrom,
    Supersedes,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GearMemoryBundle {
    pub format: String,
    #[serde(default)]
    pub source_refs: Vec<SourceRef>,
    #[serde(default)]
    pub memory_entries: Vec<MemoryEntry>,
    #[serde(default)]
    pub event_log_entries: Vec<EventLogEntry>,
    #[serde(default)]
    pub code_maps: Vec<CodeMap>,
    #[serde(default)]
    pub provenance_records: Vec<ProvenanceRecord>,
}

impl GearMemoryBundle {
    pub fn validate(&self) -> Result<(), ContractValidationError> {
        if self.format != "gear.memory.v0.1" {
            return Err(ContractValidationError::InvalidSchemaVersion {
                field: "format",
                value: self.format.clone(),
            });
        }

        for source in &self.source_refs {
            source.validate()?;
        }
        for entry in &self.memory_entries {
            entry.validate()?;
        }
        for event in &self.event_log_entries {
            event.validate()?;
        }
        for code_map in &self.code_maps {
            code_map.validate()?;
        }
        for provenance in &self.provenance_records {
            provenance.validate()?;
        }

        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SafeMetadata {
    #[serde(flatten)]
    values: BTreeMap<String, Value>,
}

impl SafeMetadata {
    pub fn from_pairs<const N: usize>(pairs: [(String, String); N]) -> Self {
        Self {
            values: pairs
                .into_iter()
                .map(|(key, value)| (key, Value::String(value)))
                .collect(),
        }
    }

    pub fn from_values(values: BTreeMap<String, Value>) -> Self {
        Self { values }
    }

    pub fn validate(&self) -> Result<(), MetadataValidationError> {
        for key in self.values.keys() {
            if is_secret_like_key(key) {
                return Err(MetadataValidationError::SecretLikeKey(key.clone()));
            }
        }

        Ok(())
    }

    pub fn stable_hash(&self) -> String {
        stable_json_hash(&self.values)
    }
}

impl fmt::Debug for SafeMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let redacted = self
            .values
            .keys()
            .map(|key| (key, "<redacted>"))
            .collect::<BTreeMap<_, _>>();

        formatter
            .debug_tuple("SafeMetadata")
            .field(&redacted)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataValidationError {
    SecretLikeKey(String),
}

impl fmt::Display for MetadataValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SecretLikeKey(key) => {
                write!(formatter, "metadata key `{key}` may contain a secret")
            }
        }
    }
}

impl std::error::Error for MetadataValidationError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractValidationError {
    EmptyField(&'static str),
    EmptyList(&'static str),
    SecretLikeMetadataKey(String),
    MalformedSha256 { field: &'static str, value: String },
    MalformedTimestamp { field: &'static str, value: String },
    InvalidSchemaVersion { field: &'static str, value: String },
    InvalidRange(&'static str),
    InvalidReference(&'static str),
    UnsafeIdentifier(&'static str),
}

impl fmt::Display for ContractValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyField(field) => write!(formatter, "field `{field}` must not be empty"),
            Self::EmptyList(field) => write!(formatter, "list `{field}` must not be empty"),
            Self::SecretLikeMetadataKey(key) => {
                write!(formatter, "metadata key `{key}` may contain a secret")
            }
            Self::MalformedSha256 { field, value } => {
                write!(formatter, "field `{field}` is not a sha256 hash: `{value}`")
            }
            Self::MalformedTimestamp { field, value } => {
                write!(formatter, "field `{field}` is not RFC3339: `{value}`")
            }
            Self::InvalidSchemaVersion { field, value } => {
                write!(
                    formatter,
                    "field `{field}` has unsupported schema/version: `{value}`"
                )
            }
            Self::InvalidRange(field) => write!(formatter, "range field `{field}` is invalid"),
            Self::InvalidReference(field) => {
                write!(formatter, "field `{field}` has an inconsistent reference")
            }
            Self::UnsafeIdentifier(field) => {
                write!(formatter, "field `{field}` is unsafe for local persistence")
            }
        }
    }
}

impl std::error::Error for ContractValidationError {}

fn is_secret_like_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase();
    let normalized = normalized.replace('-', "_");

    normalized == "secret"
        || normalized == "token"
        || normalized == "password"
        || normalized == "credential"
        || normalized == "api_key"
        || normalized == "raw_log"
        || normalized.ends_with("_secret")
        || normalized.ends_with("_token")
        || normalized.ends_with("_password")
        || normalized.ends_with("_credential")
        || normalized.ends_with("_api_key")
        || normalized.ends_with("_raw_log")
        || normalized.contains("secret_value")
        || normalized.contains("token_value")
        || normalized.contains("password_value")
        || normalized.contains("credential_value")
        || normalized.contains("api_key_value")
        || normalized.contains("raw_log_value")
}

fn validate_sha256_field(field: &'static str, value: &str) -> Result<(), ContractValidationError> {
    if is_valid_sha256(value) {
        return Ok(());
    }

    Err(ContractValidationError::MalformedSha256 {
        field,
        value: value.to_string(),
    })
}

fn validate_timestamp_field(
    field: &'static str,
    value: &str,
) -> Result<(), ContractValidationError> {
    if OffsetDateTime::parse(value, &Rfc3339).is_ok() {
        return Ok(());
    }

    Err(ContractValidationError::MalformedTimestamp {
        field,
        value: value.to_string(),
    })
}

fn validate_non_empty_field(
    field: &'static str,
    value: &str,
) -> Result<(), ContractValidationError> {
    if value.trim().is_empty() {
        return Err(ContractValidationError::EmptyField(field));
    }

    Ok(())
}

fn validate_storage_id(field: &'static str, value: &str) -> Result<(), ContractValidationError> {
    validate_non_empty_field(field, value)?;
    if matches!(value, "." | "..")
        || value.contains('/')
        || value.contains('\\')
        || value.contains('\0')
    {
        return Err(ContractValidationError::UnsafeIdentifier(field));
    }
    Ok(())
}

fn validate_non_empty_list<T>(
    field: &'static str,
    values: &[T],
) -> Result<(), ContractValidationError> {
    if values.is_empty() {
        return Err(ContractValidationError::EmptyList(field));
    }

    Ok(())
}

fn validate_metadata(metadata: &SafeMetadata) -> Result<(), ContractValidationError> {
    metadata.validate().map_err(|error| match error {
        MetadataValidationError::SecretLikeKey(key) => {
            ContractValidationError::SecretLikeMetadataKey(key)
        }
    })
}

fn is_valid_sha256(value: &str) -> bool {
    const PREFIX: &str = "sha256:";
    let Some(hex) = value.strip_prefix(PREFIX) else {
        return false;
    };

    hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn stable_json_hash<T>(value: &T) -> String
where
    T: Serialize,
{
    let canonical_json = serde_json::to_string(value).expect("serializable contract value");
    let digest = Sha256::digest(canonical_json.as_bytes());

    format!("sha256:{}", to_lower_hex(&digest))
}

fn to_lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);

    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }

    output
}

/// Stage 0 store error type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    IoError(String),
    SerializationError(String),
    DeserializationError(String),
    NotFound(String),
    InvalidOperation(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IoError(msg) => write!(f, "IO error: {}", msg),
            Self::SerializationError(msg) => write!(f, "serialization error: {}", msg),
            Self::DeserializationError(msg) => write!(f, "deserialization error: {}", msg),
            Self::NotFound(msg) => write!(f, "not found: {}", msg),
            Self::InvalidOperation(msg) => write!(f, "invalid operation: {}", msg),
        }
    }
}

impl std::error::Error for StoreError {}

/// Stage 0 store trait for local persistence.
pub trait Store {
    fn put_source_ref(&self, source: &SourceRef) -> Result<(), StoreError>;
    fn get_source_ref(&self, source_id: &str) -> Result<Option<SourceRef>, StoreError>;

    fn put_memory_entry(&self, entry: &MemoryEntry) -> Result<(), StoreError>;
    fn get_memory_entry(&self, memory_entry_id: &str) -> Result<Option<MemoryEntry>, StoreError>;

    fn put_provenance_record(&self, record: &ProvenanceRecord) -> Result<(), StoreError>;
    fn get_provenance_record(
        &self,
        provenance_id: &str,
    ) -> Result<Option<ProvenanceRecord>, StoreError>;

    fn put_event_log_entry(&self, event: &EventLogEntry) -> Result<(), StoreError>;
    fn get_event_log_entry(&self, event_id: &str) -> Result<Option<EventLogEntry>, StoreError>;

    /// Apply source state, provenance, and event as one idempotent logical mutation.
    fn apply_custody_mutation(
        &self,
        mutation: &CustodyMutation,
    ) -> Result<MutationReceipt, StoreError>;

    fn put_code_map(&self, code_map: &CodeMap) -> Result<(), StoreError>;
    fn get_code_map(&self, code_map_id: &str) -> Result<Option<CodeMap>, StoreError>;

    fn lookup_source_refs_by_id(&self, source_id: &str) -> Result<Vec<SourceRef>, StoreError>;
    fn lookup_source_refs_by_content_hash(
        &self,
        content_hash: &str,
    ) -> Result<Vec<SourceRef>, StoreError>;
    fn lookup_source_refs_by_origin_product(
        &self,
        origin_product: &str,
    ) -> Result<Vec<SourceRef>, StoreError>;
    fn lookup_source_refs_by_state(
        &self,
        state: &SourceState,
    ) -> Result<Vec<SourceRef>, StoreError>;
    fn lookup_source_refs_by_timestamp_range(
        &self,
        start: &str,
        end: &str,
    ) -> Result<Vec<SourceRef>, StoreError>;
    fn lookup_source_refs_by_canonical_title(
        &self,
        title_prefix: &str,
    ) -> Result<Vec<SourceRef>, StoreError>;

    fn lookup_memory_entries_by_state(
        &self,
        state: &IndexState,
    ) -> Result<Vec<MemoryEntry>, StoreError>;
    fn list_all_provenance_records(&self) -> Result<Vec<ProvenanceRecord>, StoreError>;

    /// RGPD erasure: flip the source to `Deleted` and leave an auditable
    /// provenance + event trail. Backend-independent by construction.
    fn mark_deleted(&self, id: &str, reason: &str, timestamp: &str) -> Result<(), StoreError> {
        erase_source(self, id, reason, timestamp, SourceState::Deleted)
    }

    /// RGPD anonymization: same trail as `mark_deleted`, target state
    /// `Anonymized`.
    fn mark_anonymized(&self, id: &str, reason: &str, timestamp: &str) -> Result<(), StoreError> {
        erase_source(self, id, reason, timestamp, SourceState::Anonymized)
    }
}

pub(crate) fn ensure_no_custody_conflict(
    current_source: Option<&SourceRef>,
    current_provenance: Option<&ProvenanceRecord>,
    current_event: Option<&EventLogEntry>,
    mutation: &CustodyMutation,
) -> Result<(), StoreError> {
    if current_provenance.is_some_and(|record| record != &mutation.provenance) {
        return Err(StoreError::InvalidOperation(format!(
            "custody mutation {} conflicts with provenance {}",
            mutation.mutation_id, mutation.provenance.provenance_id
        )));
    }
    if current_event.is_some_and(|event| event != &mutation.event) {
        return Err(StoreError::InvalidOperation(format!(
            "custody mutation {} conflicts with event {}",
            mutation.mutation_id, mutation.event.event_id
        )));
    }

    match (current_source, mutation.expected_state) {
        (None, Some(_)) => Err(StoreError::NotFound(format!(
            "source {} not found for custody mutation",
            mutation.source.source_id
        ))),
        (Some(current), Some(expected))
            if current != &mutation.source && current.state != expected =>
        {
            Err(StoreError::InvalidOperation(format!(
                "custody mutation {} expected source state {:?}, found {:?}",
                mutation.mutation_id, expected, current.state
            )))
        }
        (Some(current), None) if current != &mutation.source => {
            Err(StoreError::InvalidOperation(format!(
                "custody mutation {} expected source {} to be absent",
                mutation.mutation_id, mutation.source.source_id
            )))
        }
        _ => Ok(()),
    }
}

fn erase_source<S: Store + ?Sized>(
    store: &S,
    id: &str,
    reason: &str,
    timestamp: &str,
    target_state: SourceState,
) -> Result<(), StoreError> {
    OffsetDateTime::parse(timestamp, &Rfc3339)
        .map_err(|e| StoreError::InvalidOperation(format!("invalid timestamp: {}", e)))?;

    let mut source = store
        .get_source_ref(id)?
        .ok_or_else(|| StoreError::NotFound(format!("source {} not found", id)))?;

    let expected_state = source.state;
    let (operation, event_type, prefix) = match target_state {
        SourceState::Deleted => (ProvenanceOperation::Deleted, "source.deleted", "deleted"),
        SourceState::Anonymized => (ProvenanceOperation::Anonymized, "source.anonymized", "anon"),
        _ => {
            return Err(StoreError::InvalidOperation(
                "erasure only targets Deleted or Anonymized".to_string(),
            ));
        }
    };

    source.state = target_state;

    // RGPD anonymization: clear any PII-bearing fields alongside state change.
    // canonical_text may contain PII — it is never resurrected after anonymization.
    if target_state == SourceState::Anonymized {
        source.canonical_text = None;
    }

    let provenance_id = format!("prov_{}_{}", prefix, id);
    let provenance = ProvenanceRecord {
        provenance_id: provenance_id.clone(),
        actor_ref: "system".to_string(),
        operation,
        inputs: vec![id.to_string()],
        outputs: vec![id.to_string()],
        tool_ref: None,
        timestamp: timestamp.to_string(),
        metadata: SafeMetadata::from_pairs([("reason".to_string(), reason.to_string())]),
    };
    let event = EventLogEntry {
        event_id: format!("evt_{}_{}", prefix, id),
        event_type: event_type.to_string(),
        actor_ref: "system".to_string(),
        target_ref: id.to_string(),
        provenance_id,
        metadata: SafeMetadata::from_pairs([("reason".to_string(), reason.to_string())]),
        created_at: timestamp.to_string(),
    };
    let mutation = CustodyMutation::new(Some(expected_state), source, provenance, event);
    store.apply_custody_mutation(&mutation).map(|_| ())
}

/// File-backed store implementation for Stage 0.
/// Format: JSON files per entity type in separate directories.
/// Directory structure:
///   {root}/
///     sources/{source_id}.json
///     memory_entries/{memory_entry_id}.json
///     provenance_records/{provenance_id}.json
///     event_log_entries/{event_id}.json
pub struct FileStore {
    root: PathBuf,
    custody_lock: Mutex<()>,
}

impl FileStore {
    /// Create a new file-backed store at the given path.
    pub fn new(root: &Path) -> Result<Self, StoreError> {
        fs::create_dir_all(root.join("sources")).map_err(|e| StoreError::IoError(e.to_string()))?;
        fs::create_dir_all(root.join("memory_entries"))
            .map_err(|e| StoreError::IoError(e.to_string()))?;
        fs::create_dir_all(root.join("provenance_records"))
            .map_err(|e| StoreError::IoError(e.to_string()))?;
        fs::create_dir_all(root.join("event_log_entries"))
            .map_err(|e| StoreError::IoError(e.to_string()))?;

        fs::create_dir_all(root.join("code_maps"))
            .map_err(|e| StoreError::IoError(e.to_string()))?;
        fs::create_dir_all(root.join("custody_mutations"))
            .map_err(|e| StoreError::IoError(e.to_string()))?;

        let store = Self {
            root: root.to_path_buf(),
            custody_lock: Mutex::new(()),
        };
        store.recover_pending_custody_mutations()?;
        Ok(store)
    }

    fn source_path(&self, source_id: &str) -> PathBuf {
        self.root
            .join("sources")
            .join(format!("{}.json", source_id))
    }

    fn memory_entry_path(&self, memory_entry_id: &str) -> PathBuf {
        self.root
            .join("memory_entries")
            .join(format!("{}.json", memory_entry_id))
    }

    fn provenance_record_path(&self, provenance_id: &str) -> PathBuf {
        self.root
            .join("provenance_records")
            .join(format!("{}.json", provenance_id))
    }

    fn event_log_entry_path(&self, event_id: &str) -> PathBuf {
        self.root
            .join("event_log_entries")
            .join(format!("{}.json", event_id))
    }

    fn code_map_path(&self, code_map_id: &str) -> PathBuf {
        self.root
            .join("code_maps")
            .join(format!("{}.json", code_map_id))
    }

    fn custody_mutation_path(&self, mutation_id: &str) -> PathBuf {
        self.root
            .join("custody_mutations")
            .join(format!("{}.json", mutation_id))
    }

    fn list_json_files(&self, dir: &Path) -> Result<Vec<PathBuf>, StoreError> {
        let mut paths = Vec::new();

        if !dir.exists() {
            return Ok(paths);
        }

        for entry in fs::read_dir(dir).map_err(|e| StoreError::IoError(e.to_string()))? {
            let entry = entry.map_err(|e| StoreError::IoError(e.to_string()))?;
            let path = entry.path();

            if path.is_file() && path.extension().map(|e| e == "json").unwrap_or(false) {
                paths.push(path);
            }
        }

        Ok(paths)
    }

    fn read_json_file<T: serde::de::DeserializeOwned>(
        &self,
        path: &Path,
    ) -> Result<Option<T>, StoreError> {
        if !path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(path).map_err(|e| StoreError::IoError(e.to_string()))?;
        let value = serde_json::from_str(&content)
            .map_err(|e| StoreError::DeserializationError(e.to_string()))?;

        Ok(Some(value))
    }

    fn write_json_file<T: serde::Serialize>(
        &self,
        path: &Path,
        value: &T,
    ) -> Result<(), StoreError> {
        let json =
            serde_json::to_vec(value).map_err(|e| StoreError::SerializationError(e.to_string()))?;
        let temporary = path.with_extension("json.pending-write");
        let mut file =
            fs::File::create(&temporary).map_err(|error| StoreError::IoError(error.to_string()))?;
        file.write_all(&json)
            .and_then(|()| file.sync_all())
            .map_err(|error| StoreError::IoError(error.to_string()))?;
        fs::rename(&temporary, path).map_err(|error| StoreError::IoError(error.to_string()))?;
        if let Some(parent) = path.parent() {
            fs::File::open(parent)
                .and_then(|directory| directory.sync_all())
                .map_err(|error| StoreError::IoError(error.to_string()))?;
        }
        Ok(())
    }

    fn recover_pending_custody_mutations(&self) -> Result<(), StoreError> {
        let mut pending = self.list_json_files(&self.root.join("custody_mutations"))?;
        pending.sort();
        for path in pending {
            let mutation = self
                .read_json_file::<CustodyMutation>(&path)?
                .ok_or_else(|| StoreError::NotFound(path.display().to_string()))?;
            self.apply_file_custody_mutation(&mutation, true)?;
        }
        Ok(())
    }

    fn apply_file_custody_mutation(
        &self,
        mutation: &CustodyMutation,
        recovering_from_journal: bool,
    ) -> Result<MutationReceipt, StoreError> {
        let _guard = self.custody_lock.lock().map_err(|_| {
            StoreError::IoError("file-store custody mutation lock poisoned".to_string())
        })?;
        mutation
            .validate()
            .map_err(|error| StoreError::InvalidOperation(error.to_string()))?;
        let journal_path = self.custody_mutation_path(&mutation.mutation_id);
        let current_source = self.get_source_ref(&mutation.source.source_id)?;
        let current_provenance = self.get_provenance_record(&mutation.provenance.provenance_id)?;
        let current_event = self.get_event_log_entry(&mutation.event.event_id)?;

        if current_source.as_ref() == Some(&mutation.source)
            && current_provenance.as_ref() == Some(&mutation.provenance)
            && current_event.as_ref() == Some(&mutation.event)
        {
            if journal_path.exists() {
                fs::remove_file(&journal_path)
                    .map_err(|error| StoreError::IoError(error.to_string()))?;
            }
            return Ok(MutationReceipt {
                mutation_id: mutation.mutation_id.clone(),
                outcome: if recovering_from_journal {
                    MutationOutcome::Recovered
                } else {
                    MutationOutcome::AlreadyApplied
                },
                records_hash: mutation.records_hash(),
            });
        }

        ensure_no_custody_conflict(
            current_source.as_ref(),
            current_provenance.as_ref(),
            current_event.as_ref(),
            mutation,
        )?;
        let partially_applied = current_source.as_ref() == Some(&mutation.source)
            || current_provenance.as_ref() == Some(&mutation.provenance)
            || current_event.as_ref() == Some(&mutation.event);

        if !recovering_from_journal {
            self.write_json_file(&journal_path, mutation)?;
        }
        self.write_json_file(
            &self.source_path(&mutation.source.source_id),
            &mutation.source,
        )?;
        self.write_json_file(
            &self.provenance_record_path(&mutation.provenance.provenance_id),
            &mutation.provenance,
        )?;
        self.write_json_file(
            &self.event_log_entry_path(&mutation.event.event_id),
            &mutation.event,
        )?;
        fs::remove_file(&journal_path).map_err(|error| StoreError::IoError(error.to_string()))?;

        Ok(MutationReceipt {
            mutation_id: mutation.mutation_id.clone(),
            outcome: if recovering_from_journal || partially_applied {
                MutationOutcome::Recovered
            } else {
                MutationOutcome::Applied
            },
            records_hash: mutation.records_hash(),
        })
    }
}

impl Store for FileStore {
    fn put_source_ref(&self, source: &SourceRef) -> Result<(), StoreError> {
        source
            .validate()
            .map_err(|e| StoreError::InvalidOperation(e.to_string()))?;

        self.write_json_file(&self.source_path(&source.source_id), source)
    }

    fn get_source_ref(&self, source_id: &str) -> Result<Option<SourceRef>, StoreError> {
        validate_storage_id("source_id", source_id)
            .map_err(|error| StoreError::InvalidOperation(error.to_string()))?;
        self.read_json_file(&self.source_path(source_id))
    }

    fn put_memory_entry(&self, entry: &MemoryEntry) -> Result<(), StoreError> {
        entry
            .validate()
            .map_err(|e| StoreError::InvalidOperation(e.to_string()))?;

        self.write_json_file(&self.memory_entry_path(&entry.memory_entry_id), entry)
    }

    fn get_memory_entry(&self, memory_entry_id: &str) -> Result<Option<MemoryEntry>, StoreError> {
        validate_storage_id("memory_entry_id", memory_entry_id)
            .map_err(|error| StoreError::InvalidOperation(error.to_string()))?;
        self.read_json_file(&self.memory_entry_path(memory_entry_id))
    }

    fn put_provenance_record(&self, record: &ProvenanceRecord) -> Result<(), StoreError> {
        record
            .validate()
            .map_err(|e| StoreError::InvalidOperation(e.to_string()))?;

        self.write_json_file(&self.provenance_record_path(&record.provenance_id), record)
    }

    fn get_provenance_record(
        &self,
        provenance_id: &str,
    ) -> Result<Option<ProvenanceRecord>, StoreError> {
        validate_storage_id("provenance_id", provenance_id)
            .map_err(|error| StoreError::InvalidOperation(error.to_string()))?;
        self.read_json_file(&self.provenance_record_path(provenance_id))
    }

    fn put_event_log_entry(&self, event: &EventLogEntry) -> Result<(), StoreError> {
        event
            .validate()
            .map_err(|e| StoreError::InvalidOperation(e.to_string()))?;

        self.write_json_file(&self.event_log_entry_path(&event.event_id), event)
    }

    fn get_event_log_entry(&self, event_id: &str) -> Result<Option<EventLogEntry>, StoreError> {
        validate_storage_id("event_id", event_id)
            .map_err(|error| StoreError::InvalidOperation(error.to_string()))?;
        self.read_json_file(&self.event_log_entry_path(event_id))
    }

    fn apply_custody_mutation(
        &self,
        mutation: &CustodyMutation,
    ) -> Result<MutationReceipt, StoreError> {
        self.apply_file_custody_mutation(mutation, false)
    }

    fn put_code_map(&self, code_map: &CodeMap) -> Result<(), StoreError> {
        code_map
            .validate()
            .map_err(|e| StoreError::InvalidOperation(e.to_string()))?;

        self.write_json_file(&self.code_map_path(&code_map.code_map_id), code_map)
    }

    fn get_code_map(&self, code_map_id: &str) -> Result<Option<CodeMap>, StoreError> {
        validate_storage_id("code_map_id", code_map_id)
            .map_err(|error| StoreError::InvalidOperation(error.to_string()))?;
        self.read_json_file(&self.code_map_path(code_map_id))
    }

    fn lookup_source_refs_by_id(&self, source_id: &str) -> Result<Vec<SourceRef>, StoreError> {
        match self.get_source_ref(source_id)? {
            Some(source) => Ok(vec![source]),
            None => Ok(vec![]),
        }
    }

    fn lookup_source_refs_by_content_hash(
        &self,
        content_hash: &str,
    ) -> Result<Vec<SourceRef>, StoreError> {
        let paths = self.list_json_files(&self.root.join("sources"))?;
        let mut results = Vec::new();

        for path in paths {
            if let Some(source) = self
                .read_json_file::<SourceRef>(&path)?
                .filter(|s| s.content_hash == content_hash)
            {
                results.push(source);
            }
        }

        Ok(results)
    }

    fn lookup_source_refs_by_origin_product(
        &self,
        origin_product: &str,
    ) -> Result<Vec<SourceRef>, StoreError> {
        let paths = self.list_json_files(&self.root.join("sources"))?;
        let mut results = Vec::new();

        for path in paths {
            if let Some(source) = self
                .read_json_file::<SourceRef>(&path)?
                .filter(|s| s.origin_product == origin_product)
            {
                results.push(source);
            }
        }

        Ok(results)
    }

    fn lookup_source_refs_by_state(
        &self,
        state: &SourceState,
    ) -> Result<Vec<SourceRef>, StoreError> {
        let paths = self.list_json_files(&self.root.join("sources"))?;
        let mut results = Vec::new();

        for path in paths {
            if let Some(source) = self
                .read_json_file::<SourceRef>(&path)?
                .filter(|s| s.state == *state)
            {
                results.push(source);
            }
        }

        Ok(results)
    }

    fn lookup_source_refs_by_timestamp_range(
        &self,
        start: &str,
        end: &str,
    ) -> Result<Vec<SourceRef>, StoreError> {
        let start_time = OffsetDateTime::parse(start, &Rfc3339)
            .map_err(|e| StoreError::InvalidOperation(format!("invalid start timestamp: {}", e)))?;
        let end_time = OffsetDateTime::parse(end, &Rfc3339)
            .map_err(|e| StoreError::InvalidOperation(format!("invalid end timestamp: {}", e)))?;

        let paths = self.list_json_files(&self.root.join("sources"))?;
        let mut results = Vec::new();

        for path in paths {
            if let Some(source) = self.read_json_file::<SourceRef>(&path)?
                && let Ok(created) = OffsetDateTime::parse(&source.created_at, &Rfc3339)
                && created >= start_time
                && created <= end_time
            {
                results.push(source);
            }
        }

        Ok(results)
    }

    fn lookup_source_refs_by_canonical_title(
        &self,
        title_prefix: &str,
    ) -> Result<Vec<SourceRef>, StoreError> {
        let paths = self.list_json_files(&self.root.join("sources"))?;
        let mut results = Vec::new();
        let search_lower = title_prefix.to_lowercase();

        for path in paths {
            if let Some(source) = self.read_json_file::<SourceRef>(&path)?
                && let Some(title) = &source.canonical_title
                && title.to_lowercase().starts_with(&search_lower)
            {
                results.push(source);
            }
        }

        Ok(results)
    }

    fn lookup_memory_entries_by_state(
        &self,
        state: &IndexState,
    ) -> Result<Vec<MemoryEntry>, StoreError> {
        let paths = self.list_json_files(&self.root.join("memory_entries"))?;
        let mut results = Vec::new();

        for path in paths {
            if let Some(entry) = self
                .read_json_file::<MemoryEntry>(&path)?
                .filter(|e| e.index_state == *state)
            {
                results.push(entry);
            }
        }

        Ok(results)
    }

    fn list_all_provenance_records(&self) -> Result<Vec<ProvenanceRecord>, StoreError> {
        let paths = self.list_json_files(&self.root.join("provenance_records"))?;
        let mut results = Vec::new();

        for path in paths {
            if let Some(record) = self.read_json_file::<ProvenanceRecord>(&path)? {
                results.push(record);
            }
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash() -> String {
        format!("sha256:{}", "a".repeat(64))
    }

    #[test]
    fn project_card_names_the_repo_and_responsibility() {
        assert_eq!(PROJECT.name, "gear-memory");
        assert!(summary().contains(PROJECT.role));
        assert!(summary().contains("never decide"));
    }

    #[test]
    fn source_ref_roundtrips_with_revoked_state() {
        let mut source = valid_source_ref();
        source.state = SourceState::Revoked;

        let encoded = serde_json::to_string(&source).expect("source serializes");
        let decoded: SourceRef = serde_json::from_str(&encoded).expect("source deserializes");

        assert_eq!(decoded, source);
    }

    #[test]
    fn memory_entry_rejects_missing_required_source_ref() {
        let payload = r#"{
            "memory_entry_id": "mem_01",
            "content_hash": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "index_state": "indexed",
            "index_metadata": {
                "schema_version": "memory-entry.v0.1",
                "chunk_count": 2,
                "embedding_model_ref": null,
                "indexed_at": "2026-06-30T00:01:00Z"
            },
            "created_at": "2026-06-30T00:00:00Z"
        }"#;

        let error =
            serde_json::from_str::<MemoryEntry>(payload).expect_err("source_ref is required");

        assert!(error.to_string().contains("missing field `source_ref`"));
    }

    #[test]
    fn event_log_debug_redacts_metadata_values() {
        let event = EventLogEntry {
            event_id: "evt_01".to_string(),
            event_type: "memory.indexed".to_string(),
            actor_ref: "actor_01".to_string(),
            target_ref: "mem_01".to_string(),
            provenance_id: "prov_01".to_string(),
            metadata: SafeMetadata::from_pairs([(
                "provider_metadata_without_secrets".to_string(),
                "token-should-not-leak".to_string(),
            )]),
            created_at: "2026-06-30T00:00:00Z".to_string(),
        };

        let debug = format!("{event:?}");

        assert!(debug.contains("provider_metadata_without_secrets"));
        assert!(!debug.contains("token-should-not-leak"));
    }

    #[test]
    fn metadata_validation_rejects_secret_like_keys() {
        let metadata = SafeMetadata::from_pairs([(
            "api_token".to_string(),
            "token-should-not-be-stored".to_string(),
        )]);

        let error = metadata
            .validate()
            .expect_err("secret-like metadata keys are rejected");

        assert_eq!(
            error,
            MetadataValidationError::SecretLikeKey("api_token".to_string())
        );
    }

    #[test]
    fn metadata_allows_safe_count_keys() {
        let metadata = SafeMetadata::from_pairs([("secret_count".to_string(), "0".to_string())]);

        metadata
            .validate()
            .expect("safe summary counts are allowed");
    }

    #[test]
    fn provenance_record_stable_hash_ignores_metadata_insertion_order() {
        let mut left = indexed_provenance_record();
        left.metadata = SafeMetadata::from_pairs([
            ("runner".to_string(), "local".to_string()),
            ("tool".to_string(), "wrench-loader".to_string()),
        ]);

        let mut right = indexed_provenance_record();
        right.metadata = SafeMetadata::from_pairs([
            ("tool".to_string(), "wrench-loader".to_string()),
            ("runner".to_string(), "local".to_string()),
        ]);

        assert_eq!(left.stable_hash(), right.stable_hash());
    }

    #[test]
    fn local_persistence_ids_reject_path_traversal() {
        let mut source = valid_source_ref();
        source.source_id = "../../outside".to_string();

        assert_eq!(
            source.validate(),
            Err(ContractValidationError::UnsafeIdentifier("source_id"))
        );

        let directory = tempfile::tempdir().expect("temp dir");
        let store = FileStore::new(directory.path()).expect("store");
        assert!(
            store.get_source_ref("../../outside").is_err(),
            "reads must not escape the store root either"
        );
    }

    #[test]
    fn custody_mutation_rejects_forged_mutation_id() {
        let source = valid_source_ref();
        let provenance = ProvenanceRecord {
            provenance_id: "prov_import_src_01".to_string(),
            actor_ref: "actor_01".to_string(),
            operation: ProvenanceOperation::Imported,
            inputs: vec![],
            outputs: vec![source.source_id.clone()],
            tool_ref: Some("gear-loader".to_string()),
            timestamp: source.created_at.clone(),
            metadata: SafeMetadata::default(),
        };
        let event = EventLogEntry {
            event_id: "evt_import_src_01".to_string(),
            event_type: "source.imported".to_string(),
            actor_ref: "actor_01".to_string(),
            target_ref: source.source_id.clone(),
            provenance_id: provenance.provenance_id.clone(),
            metadata: SafeMetadata::default(),
            created_at: source.created_at.clone(),
        };
        let mut mutation = CustodyMutation::new(None, source, provenance, event);
        mutation.mutation_id = "custody_forged".to_string();

        assert_eq!(
            mutation.validate(),
            Err(ContractValidationError::InvalidReference(
                "mutation.mutation_id"
            ))
        );
    }

    #[test]
    fn source_ref_validation_rejects_malformed_content_hash() {
        let mut source = valid_source_ref();
        source.content_hash = "sha256:not-hex".to_string();

        let error = source
            .validate()
            .expect_err("malformed content hash is rejected");

        assert_eq!(
            error,
            ContractValidationError::MalformedSha256 {
                field: "content_hash",
                value: "sha256:not-hex".to_string()
            }
        );
    }

    #[test]
    fn memory_entry_validation_rejects_wrong_schema_version() {
        let mut entry = valid_memory_entry();
        entry.index_metadata.schema_version = "memory-entry.v9".to_string();

        let error = entry
            .validate()
            .expect_err("wrong schema version is rejected");

        assert_eq!(
            error,
            ContractValidationError::InvalidSchemaVersion {
                field: "index_metadata.schema_version",
                value: "memory-entry.v9".to_string()
            }
        );
    }

    #[test]
    fn code_map_validation_rejects_invalid_symbol_range() {
        let mut code_map = valid_code_map();
        code_map.symbols[0].range.end_line = 0;

        let error = code_map
            .validate()
            .expect_err("invalid symbol range is rejected");

        assert_eq!(error, ContractValidationError::InvalidRange("end_line"));
    }

    #[test]
    fn bundle_validation_accepts_p0_contract_family() {
        let bundle = GearMemoryBundle {
            format: "gear.memory.v0.1".to_string(),
            source_refs: vec![valid_source_ref()],
            memory_entries: vec![valid_memory_entry()],
            event_log_entries: vec![valid_event_log_entry()],
            code_maps: vec![valid_code_map()],
            provenance_records: vec![indexed_provenance_record()],
        };

        bundle.validate().expect("valid bundle is accepted");
    }

    #[test]
    fn bundle_validation_rejects_unknown_format() {
        let bundle = GearMemoryBundle {
            format: "gear.memory.v9".to_string(),
            source_refs: vec![],
            memory_entries: vec![],
            event_log_entries: vec![],
            code_maps: vec![],
            provenance_records: vec![],
        };

        let error = bundle.validate().expect_err("unknown format is rejected");

        assert_eq!(
            error,
            ContractValidationError::InvalidSchemaVersion {
                field: "format",
                value: "gear.memory.v9".to_string()
            }
        );
    }

    #[test]
    fn file_store_recovers_after_source_write() {
        assert_file_store_recovers_custody_mutation(1);
    }

    #[test]
    fn file_store_recovers_after_provenance_write() {
        assert_file_store_recovers_custody_mutation(2);
    }

    #[test]
    fn file_store_recovers_after_event_write_before_journal_cleanup() {
        assert_file_store_recovers_custody_mutation(3);
    }

    fn assert_file_store_recovers_custody_mutation(applied_records: usize) {
        let directory = tempfile::tempdir().expect("temp dir");
        let mutation = deletion_mutation();

        {
            let store = FileStore::new(directory.path()).expect("store");
            store
                .put_source_ref(&valid_source_ref())
                .expect("initial source");
            store
                .write_json_file(
                    &store.custody_mutation_path(&mutation.mutation_id),
                    &mutation,
                )
                .expect("write intent journal");
            if applied_records >= 1 {
                store
                    .write_json_file(&store.source_path("src_01"), &mutation.source)
                    .expect("simulate source write");
            }
            if applied_records >= 2 {
                store
                    .write_json_file(
                        &store.provenance_record_path(&mutation.provenance.provenance_id),
                        &mutation.provenance,
                    )
                    .expect("simulate provenance write");
            }
            if applied_records >= 3 {
                store
                    .write_json_file(
                        &store.event_log_entry_path(&mutation.event.event_id),
                        &mutation.event,
                    )
                    .expect("simulate event write");
            }
        }

        let recovered = FileStore::new(directory.path()).expect("recovery succeeds");
        assert_eq!(
            recovered.get_source_ref("src_01").unwrap(),
            Some(mutation.source.clone())
        );
        assert_eq!(
            recovered
                .get_provenance_record(&mutation.provenance.provenance_id)
                .unwrap(),
            Some(mutation.provenance.clone())
        );
        assert_eq!(
            recovered
                .get_event_log_entry(&mutation.event.event_id)
                .unwrap(),
            Some(mutation.event.clone())
        );
        assert!(
            !recovered
                .custody_mutation_path(&mutation.mutation_id)
                .exists()
        );
    }

    fn deletion_mutation() -> CustodyMutation {
        let mut target = valid_source_ref();
        target.state = SourceState::Deleted;
        let provenance = ProvenanceRecord {
            provenance_id: "prov_deleted_src_01".to_string(),
            actor_ref: "system".to_string(),
            operation: ProvenanceOperation::Deleted,
            inputs: vec!["src_01".to_string()],
            outputs: vec!["src_01".to_string()],
            tool_ref: None,
            timestamp: "2026-07-01T00:00:00Z".to_string(),
            metadata: SafeMetadata::from_pairs([(
                "reason".to_string(),
                "recovery test".to_string(),
            )]),
        };
        let event = EventLogEntry {
            event_id: "evt_deleted_src_01".to_string(),
            event_type: "source.deleted".to_string(),
            actor_ref: "system".to_string(),
            target_ref: "src_01".to_string(),
            provenance_id: provenance.provenance_id.clone(),
            metadata: provenance.metadata.clone(),
            created_at: "2026-07-01T00:00:00Z".to_string(),
        };
        CustodyMutation::new(Some(SourceState::Active), target, provenance, event)
    }

    fn valid_source_ref() -> SourceRef {
        SourceRef {
            source_id: "src_01".to_string(),
            source_type: SourceType::Document,
            origin_product: "wrench-loader".to_string(),
            uri: Some("file:///tmp/source.md".to_string()),
            content_hash: hash(),
            provenance_id: "prov_01".to_string(),
            state: SourceState::Active,
            created_at: "2026-06-30T00:00:00Z".to_string(),
            canonical_title: None,
            canonical_text: None,
            metadata: SafeMetadata::default(),
        }
    }

    fn valid_memory_entry() -> MemoryEntry {
        MemoryEntry {
            memory_entry_id: "mem_01".to_string(),
            source_ref: "src_01".to_string(),
            content_hash: hash(),
            index_state: IndexState::Indexed,
            index_metadata: IndexMetadata {
                schema_version: "memory-entry.v0.1".to_string(),
                chunk_count: 1,
                embedding_model_ref: None,
                indexed_at: Some("2026-06-30T00:01:00Z".to_string()),
            },
            created_at: "2026-06-30T00:00:00Z".to_string(),
        }
    }

    fn indexed_provenance_record() -> ProvenanceRecord {
        ProvenanceRecord {
            provenance_id: "prov_01".to_string(),
            actor_ref: "actor_01".to_string(),
            operation: ProvenanceOperation::Indexed,
            inputs: vec!["src_01".to_string()],
            outputs: vec!["mem_01".to_string()],
            tool_ref: Some("gear-memory".to_string()),
            timestamp: "2026-06-30T00:00:00Z".to_string(),
            metadata: SafeMetadata::from_pairs([("runner".to_string(), "local".to_string())]),
        }
    }

    fn valid_event_log_entry() -> EventLogEntry {
        EventLogEntry {
            event_id: "evt_01".to_string(),
            event_type: "memory.indexed".to_string(),
            actor_ref: "actor_01".to_string(),
            target_ref: "mem_01".to_string(),
            provenance_id: "prov_01".to_string(),
            metadata: SafeMetadata::from_pairs([("result".to_string(), "ok".to_string())]),
            created_at: "2026-06-30T00:00:00Z".to_string(),
        }
    }

    fn valid_code_map() -> CodeMap {
        CodeMap {
            code_map_id: "cm_01".to_string(),
            root_source_ref: "src_01".to_string(),
            scope: CodeMapScope {
                repo_ref: Some("repo_demo".to_string()),
                revision: "git:abc123".to_string(),
                paths: vec!["src/".to_string()],
            },
            parser_refs: vec!["tree-sitter:rust@0.0.0-demo".to_string()],
            symbols: vec![CodeSymbol {
                symbol_id: "sym_01".to_string(),
                kind: CodeSymbolKind::Function,
                name: "demo::main".to_string(),
                source_ref: "src_01".to_string(),
                range: SourceRange {
                    start_line: 1,
                    end_line: 3,
                },
                content_hash: hash(),
            }],
            edges: vec![],
            state: CodeMapState::Active,
            created_at: "2026-06-30T00:00:00Z".to_string(),
        }
    }
}
