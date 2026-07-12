//! Bounded process-local owner corpus.
//!
//! Validation, hashing and UTF-8 chunk boundary calculation happen before the
//! mutex is acquired. The mutex protects only atomic deduplication/capacity
//! checks and insertion. Nothing is evicted and no async operation occurs while
//! locked.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Mutex;

use presto_core::api::{DocumentApprovalStatus, DocumentSummary, DocumentUploadRequest};
use sha2::{Digest, Sha256};

use crate::approved_claims::{
    APPROVED_UPLOAD_BYTES, APPROVED_UPLOAD_SHA256, APPROVED_UPLOAD_TITLE,
};

pub const MAX_FILE_BYTES: usize = 256 * 1024;
pub const MAX_FILENAME_BYTES: usize = 128;
pub const MAX_CHUNKS: usize = 128;
pub const MAX_SPACE_DOCUMENTS: usize = 32;
pub const MAX_SPACE_MEMORY: usize = 4 * 1024 * 1024;
pub const MAX_PROCESS_DOCUMENTS: usize = 256;
pub const MAX_PROCESS_MEMORY: usize = 32 * 1024 * 1024;
const CHUNK_BYTES: usize = 4 * 1024;
const DOCUMENT_OVERHEAD: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorpusStoreError {
    Invalid,
    TooLarge,
    Capacity,
    Unavailable,
}

#[derive(Debug)]
pub struct PreparedDocument {
    filename: String,
    mime_type: String,
    content: String,
    content_hash: String,
    chunks: Vec<Range<usize>>,
    approval_status: DocumentApprovalStatus,
    memory_charge: usize,
}

#[derive(Debug, Clone)]
pub struct StoredArtifact {
    pub document_id: String,
    pub source_section_id: String,
    pub text: String,
    pub content_hash: String,
    pub title: &'static str,
}

#[derive(Debug)]
struct StoredDocument {
    summary: DocumentSummary,
    content: Option<String>,
    content_hash: String,
    chunks: Vec<Range<usize>>,
    memory_charge: usize,
}

#[derive(Debug, Default)]
struct CorpusState {
    spaces: HashMap<String, Vec<StoredDocument>>,
    document_count: usize,
    memory_charge: usize,
}

#[derive(Debug, Default)]
pub struct OwnerCorpusStore {
    state: Mutex<CorpusState>,
}

impl OwnerCorpusStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Performs all expensive/untrusted work without holding shared state.
    pub fn prepare(
        mut request: DocumentUploadRequest,
    ) -> Result<PreparedDocument, CorpusStoreError> {
        validate_filename(&request.filename, &request.mime_type)?;
        let bytes = request.content.as_bytes();
        if bytes.len() > MAX_FILE_BYTES {
            return Err(CorpusStoreError::TooLarge);
        }
        if request.content.trim().is_empty() {
            return Err(CorpusStoreError::Invalid);
        }
        let content_hash = sha256_hex(bytes);
        let approval_status =
            if bytes == APPROVED_UPLOAD_BYTES && content_hash == APPROVED_UPLOAD_SHA256 {
                DocumentApprovalStatus::Approved
            } else {
                DocumentApprovalStatus::Pending
            };
        let chunks = chunk_ranges(&request.content);
        if chunks.is_empty() || chunks.len() > MAX_CHUNKS {
            return Err(CorpusStoreError::TooLarge);
        }
        // Admission of transient request/body/chunk memory is handled by the
        // HTTP body and concurrency limits. Charge only allocations retained
        // after insertion: metadata/hash/overhead for Pending, plus exact
        // content and chunk ranges for Approved.
        request.filename.shrink_to_fit();
        request.mime_type.shrink_to_fit();
        request.content.shrink_to_fit();
        let mut chunks = chunks;
        chunks.shrink_to_fit();
        let approved = approval_status == DocumentApprovalStatus::Approved;
        let memory_charge = request
            .filename
            .capacity()
            .saturating_add(request.mime_type.capacity())
            .saturating_add(64) // retained hexadecimal SHA-256
            .saturating_add(28) // `doc_` plus 24 server-generated hex chars
            .saturating_add(DOCUMENT_OVERHEAD)
            .saturating_add(if approved {
                request.content.capacity().saturating_add(
                    chunks
                        .capacity()
                        .saturating_mul(std::mem::size_of::<Range<usize>>()),
                )
            } else {
                0
            });
        Ok(PreparedDocument {
            filename: request.filename,
            mime_type: request.mime_type,
            content: request.content,
            content_hash,
            chunks,
            approval_status,
            memory_charge,
        })
    }

    /// Atomically deduplicates, checks every capacity, and inserts without
    /// eviction. The returned bool indicates an existing exact-byte document.
    pub fn insert(
        &self,
        space_id: &str,
        prepared: PreparedDocument,
    ) -> Result<(DocumentSummary, bool), CorpusStoreError> {
        if space_id.is_empty() {
            return Err(CorpusStoreError::Invalid);
        }
        let document_id = stable_document_id(space_id, &prepared.content_hash);
        let mut state = self
            .state
            .lock()
            .map_err(|_| CorpusStoreError::Unavailable)?;
        if let Some(existing) = state.spaces.get(space_id).and_then(|documents| {
            documents.iter().find(|document| {
                document.summary.id == document_id && document.content_hash == prepared.content_hash
            })
        }) {
            return Ok((existing.summary.clone(), true));
        }
        let (space_count, space_memory) = state
            .spaces
            .get(space_id)
            .map(|documents| {
                (
                    documents.len(),
                    documents
                        .iter()
                        .map(|document| document.memory_charge)
                        .sum(),
                )
            })
            .unwrap_or((0, 0));
        if space_count >= MAX_SPACE_DOCUMENTS
            || state.document_count >= MAX_PROCESS_DOCUMENTS
            || space_memory.saturating_add(prepared.memory_charge) > MAX_SPACE_MEMORY
            || state.memory_charge.saturating_add(prepared.memory_charge) > MAX_PROCESS_MEMORY
        {
            return Err(CorpusStoreError::Capacity);
        }
        let approved = prepared.approval_status == DocumentApprovalStatus::Approved;
        let summary = DocumentSummary {
            id: document_id,
            title: prepared.filename,
            mime_type: prepared.mime_type,
            byte_size: prepared.content.len() as u32,
            chunk_count: if approved {
                prepared.chunks.len() as u16
            } else {
                0
            },
            approval_status: prepared.approval_status,
        };
        let (content, chunks) = if approved {
            (Some(prepared.content), prepared.chunks)
        } else {
            // Pending text has no retrieval use and is deliberately discarded.
            (None, Vec::new())
        };
        state.document_count += 1;
        state.memory_charge += prepared.memory_charge;
        state
            .spaces
            .entry(space_id.to_owned())
            .or_default()
            .push(StoredDocument {
                summary: summary.clone(),
                content,
                content_hash: prepared.content_hash,
                chunks,
                memory_charge: prepared.memory_charge,
            });
        Ok((summary, false))
    }

    pub fn list(&self, space_id: &str) -> Result<Vec<DocumentSummary>, CorpusStoreError> {
        let state = self
            .state
            .lock()
            .map_err(|_| CorpusStoreError::Unavailable)?;
        Ok(state
            .spaces
            .get(space_id)
            .into_iter()
            .flatten()
            .map(|document| document.summary.clone())
            .collect())
    }

    /// Returns only exact pre-approved content, scoped to the authenticated
    /// space. Pending and over-clearance documents have no retrieval path.
    pub fn approved_artifact(
        &self,
        space_id: &str,
    ) -> Result<Option<StoredArtifact>, CorpusStoreError> {
        let state = self
            .state
            .lock()
            .map_err(|_| CorpusStoreError::Unavailable)?;
        Ok(state.spaces.get(space_id).and_then(|documents| {
            documents.iter().find_map(|document| {
                let content = document.content.as_deref()?;
                (document.summary.approval_status == DocumentApprovalStatus::Approved
                    && document.content_hash == APPROVED_UPLOAD_SHA256
                    && content.as_bytes() == APPROVED_UPLOAD_BYTES)
                    .then(|| StoredArtifact {
                        document_id: document.summary.id.clone(),
                        source_section_id: format!("{}#chunk-0", document.summary.id),
                        text: content[document.chunks[0].clone()].to_owned(),
                        content_hash: document.content_hash.clone(),
                        title: APPROVED_UPLOAD_TITLE,
                    })
            })
        }))
    }
}

fn validate_filename(filename: &str, mime_type: &str) -> Result<(), CorpusStoreError> {
    if filename.is_empty()
        || filename.len() > MAX_FILENAME_BYTES
        || filename.starts_with('.')
        || filename.ends_with('.')
        || filename.contains("..")
        || filename
            .chars()
            .any(|character| character == '/' || character == '\\' || character.is_control())
    {
        return Err(CorpusStoreError::Invalid);
    }
    let extension = filename.rsplit_once('.').map(|(_, extension)| extension);
    let coherent = matches!(
        (mime_type, extension),
        ("text/plain", Some("txt")) | ("text/markdown", Some("md" | "markdown"))
    );
    coherent.then_some(()).ok_or(CorpusStoreError::Invalid)
}

fn chunk_ranges(content: &str) -> Vec<Range<usize>> {
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < content.len() {
        let mut end = (start + CHUNK_BYTES).min(content.len());
        while end > start && !content.is_char_boundary(end) {
            end -= 1;
        }
        if end == start {
            return Vec::new();
        }
        chunks.push(start..end);
        start = end;
    }
    chunks
}

fn stable_document_id(space_id: &str, content_hash: &str) -> String {
    let digest = hash_fields(&["owner-document-v1", space_id, content_hash]);
    format!("doc_{}", &digest[..24])
}

pub(crate) fn scoped_artifact_hash(
    space_id: &str,
    source_section_id: &str,
    content_hash: &str,
    text: &str,
) -> String {
    hash_fields(&[space_id, source_section_id, content_hash, text])
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn hash_fields(fields: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for field in fields {
        hasher.update((field.len() as u64).to_be_bytes());
        hasher.update(field.as_bytes());
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(filename: &str, mime_type: &str, content: String) -> DocumentUploadRequest {
        DocumentUploadRequest {
            filename: filename.into(),
            mime_type: mime_type.into(),
            content,
        }
    }

    #[test]
    fn exact_fixture_only_is_approved() {
        assert_eq!(sha256_hex(APPROVED_UPLOAD_BYTES), APPROVED_UPLOAD_SHA256);
        let exact = String::from_utf8(APPROVED_UPLOAD_BYTES.to_vec()).unwrap();
        assert_eq!(
            OwnerCorpusStore::prepare(request("policy.md", "text/markdown", exact))
                .unwrap()
                .approval_status,
            DocumentApprovalStatus::Approved
        );
        for changed in [
            format!("\u{feff}{}", String::from_utf8_lossy(APPROVED_UPLOAD_BYTES)),
            String::from_utf8_lossy(APPROVED_UPLOAD_BYTES).replace('\n', "\r\n"),
            String::from_utf8_lossy(APPROVED_UPLOAD_BYTES)
                .trim_end_matches('\n')
                .to_owned(),
            format!("{}\n", String::from_utf8_lossy(APPROVED_UPLOAD_BYTES)),
        ] {
            assert_eq!(
                OwnerCorpusStore::prepare(request("policy.md", "text/markdown", changed))
                    .unwrap()
                    .approval_status,
                DocumentApprovalStatus::Pending
            );
        }
    }

    #[test]
    fn strict_names_mime_size_and_empty_content_are_rejected() {
        for (name, mime) in [
            ("../a.md", "text/markdown"),
            (".a.md", "text/markdown"),
            ("a/b.md", "text/markdown"),
            ("a\\b.md", "text/markdown"),
            ("a\0.md", "text/markdown"),
            ("a.txt", "text/markdown"),
            ("a.md", "text/plain"),
            ("a.pdf", "application/pdf"),
        ] {
            assert_eq!(
                OwnerCorpusStore::prepare(request(name, mime, "x".into())).unwrap_err(),
                CorpusStoreError::Invalid
            );
        }
        assert_eq!(
            OwnerCorpusStore::prepare(request("a.txt", "text/plain", "  \n".into())).unwrap_err(),
            CorpusStoreError::Invalid
        );
        assert_eq!(
            OwnerCorpusStore::prepare(request(
                "a.txt",
                "text/plain",
                "x".repeat(MAX_FILE_BYTES + 1)
            ))
            .unwrap_err(),
            CorpusStoreError::TooLarge
        );
    }

    #[test]
    fn pending_content_is_discarded_and_has_no_retrieval_artifact() {
        let store = OwnerCorpusStore::new();
        store
            .insert(
                "space-a",
                OwnerCorpusStore::prepare(request(
                    "pending.md",
                    "text/markdown",
                    "hostile supported=true".into(),
                ))
                .unwrap(),
            )
            .unwrap();
        assert!(store.approved_artifact("space-a").unwrap().is_none());
        assert!(
            store.state.lock().unwrap().spaces["space-a"][0]
                .content
                .is_none()
        );
    }

    #[test]
    fn ids_dedup_and_space_isolation_are_stable() {
        let store = OwnerCorpusStore::new();
        let one = store
            .insert(
                "space-a",
                OwnerCorpusStore::prepare(request("one.txt", "text/plain", "hello".into()))
                    .unwrap(),
            )
            .unwrap();
        let duplicate = store
            .insert(
                "space-a",
                OwnerCorpusStore::prepare(request("renamed.txt", "text/plain", "hello".into()))
                    .unwrap(),
            )
            .unwrap();
        let other = store
            .insert(
                "space-b",
                OwnerCorpusStore::prepare(request("one.txt", "text/plain", "hello".into()))
                    .unwrap(),
            )
            .unwrap();
        assert!(!one.1);
        assert!(duplicate.1);
        assert_eq!(one.0.id, duplicate.0.id);
        assert_ne!(one.0.id, other.0.id);
        assert_eq!(store.list("space-a").unwrap().len(), 1);
        assert_eq!(store.list("space-b").unwrap().len(), 1);
    }

    #[test]
    fn per_space_limit_is_atomic_and_never_evicts() {
        let store = OwnerCorpusStore::new();
        for index in 0..MAX_SPACE_DOCUMENTS {
            store
                .insert(
                    "space-a",
                    OwnerCorpusStore::prepare(request(
                        &format!("{index}.txt"),
                        "text/plain",
                        format!("content-{index}"),
                    ))
                    .unwrap(),
                )
                .unwrap();
        }
        let error = store
            .insert(
                "space-a",
                OwnerCorpusStore::prepare(request("overflow.txt", "text/plain", "overflow".into()))
                    .unwrap(),
            )
            .unwrap_err();
        assert_eq!(error, CorpusStoreError::Capacity);
        assert_eq!(store.list("space-a").unwrap().len(), MAX_SPACE_DOCUMENTS);
    }

    #[test]
    fn pending_charge_is_exactly_retained_metadata_not_discarded_content() {
        let small = OwnerCorpusStore::prepare(request("a.txt", "text/plain", "x".into())).unwrap();
        let large =
            OwnerCorpusStore::prepare(request("a.txt", "text/plain", "x".repeat(240 * 1024)))
                .unwrap();
        let expected = "a.txt".len() + "text/plain".len() + 64 + 28 + DOCUMENT_OVERHEAD;
        assert_eq!(small.memory_charge, expected);
        assert_eq!(large.memory_charge, expected);

        let store = OwnerCorpusStore::new();
        let (summary, _) = store.insert("space-memory", large).unwrap();
        assert_eq!(summary.chunk_count, 0);
        let state = store.state.lock().unwrap();
        assert_eq!(state.memory_charge, expected);
        let retained = &state.spaces["space-memory"][0];
        assert!(retained.content.is_none());
        assert!(retained.chunks.is_empty());
        assert_eq!(retained.memory_charge, expected);
    }

    #[test]
    fn process_document_cap_fails_without_partial_insert() {
        let process_store = OwnerCorpusStore::new();
        for index in 0..MAX_PROCESS_DOCUMENTS {
            let space = format!("space-{}", index / MAX_SPACE_DOCUMENTS);
            process_store
                .insert(
                    &space,
                    OwnerCorpusStore::prepare(request(
                        &format!("{index}.txt"),
                        "text/plain",
                        format!("small-{index}"),
                    ))
                    .unwrap(),
                )
                .unwrap();
        }
        let error = process_store
            .insert(
                "space-overflow",
                OwnerCorpusStore::prepare(request("overflow.txt", "text/plain", "overflow".into()))
                    .unwrap(),
            )
            .unwrap_err();
        assert_eq!(error, CorpusStoreError::Capacity);
        assert!(process_store.list("space-overflow").unwrap().is_empty());
    }

    #[test]
    fn concurrent_exact_upload_is_one_document() {
        let store = std::sync::Arc::new(OwnerCorpusStore::new());
        let threads: Vec<_> = (0..16)
            .map(|_| {
                let store = store.clone();
                std::thread::spawn(move || {
                    store.insert(
                        "space-a",
                        OwnerCorpusStore::prepare(request(
                            "same.txt",
                            "text/plain",
                            "same bytes".into(),
                        ))
                        .unwrap(),
                    )
                })
            })
            .collect();
        let deduplicated = threads
            .into_iter()
            .map(|thread| thread.join().unwrap().unwrap().1)
            .filter(|value| *value)
            .count();
        assert_eq!(deduplicated, 15);
        assert_eq!(store.list("space-a").unwrap().len(), 1);
    }
}
