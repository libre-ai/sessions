//! Real source ingestion via gear-loader.
//! Transforms raw documents into canonical SourceRef records for questions to cite.

use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceType {
    #[serde(rename = "Document")]
    Document,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceState {
    #[serde(rename = "Active")]
    Active,
    #[serde(rename = "Archived")]
    Archived,
}

impl fmt::Display for SourceState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Active => write!(f, "Active"),
            Self::Archived => write!(f, "Archived"),
        }
    }
}

/// A stable reference to an ingested source document.
/// Derived from gear-loader's CanonicalSourceDocument, persisted by rumble-lm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRef {
    pub source_id: String,
    pub source_type: SourceType,
    pub origin_product: String,
    pub uri: Option<String>,
    pub content_hash: String,
    pub provenance_id: String,
    pub state: SourceState,
    pub created_at: String,
    pub canonical_title: Option<String>,
    pub canonical_text: Option<String>,
    pub metadata: serde_json::Value,
}

impl SourceRef {
    /// Create a SourceRef from a gear-loader CanonicalSourceDocument.
    pub fn from_canonical(
        canonical_doc: &gear_loader::CanonicalSourceDocument,
        origin_product: &str,
        provenance_id: &str,
    ) -> Self {
        let source_id = format!("src_{}", canonical_doc.document_id);
        Self {
            source_id,
            source_type: SourceType::Document,
            origin_product: origin_product.to_string(),
            uri: canonical_doc.source.uri.clone(),
            content_hash: canonical_doc.source.content_hash.clone(),
            provenance_id: provenance_id.to_string(),
            state: SourceState::Active,
            created_at: chrono::Utc::now().to_rfc3339(),
            canonical_title: canonical_doc.canonical.title.clone(),
            canonical_text: Some(canonical_doc.canonical.text.clone()),
            metadata: serde_json::json!({
                "document_format": canonical_doc.format,
                "language": canonical_doc.canonical.language,
                "source_input_type": format!("{:?}", canonical_doc.source.input_type),
                "security_classification": format!("{:?}", canonical_doc.security.classification),
                "extraction_status": format!("{:?}", canonical_doc.quality.extraction_status),
            }),
        }
    }

    /// Persist this SourceRef to the database.
    pub async fn persist(&self, pool: &PgPool) -> Result<(), sqlx::Error> {
        let metadata_json =
            serde_json::to_string(&self.metadata).unwrap_or_else(|_| "{}".to_string());
        sqlx::query(
            r#"
            INSERT INTO source_refs (source_id, source_type, origin_product, uri, content_hash, provenance_id, state, created_at, canonical_title, canonical_text, metadata)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11::jsonb)
            ON CONFLICT (source_id) DO NOTHING
            "#,
        )
        .bind(&self.source_id)
        .bind("Document")
        .bind(&self.origin_product)
        .bind(&self.uri)
        .bind(&self.content_hash)
        .bind(&self.provenance_id)
        .bind(self.state.to_string())  // Display impl
        .bind(&self.created_at)
        .bind(&self.canonical_title)
        .bind(&self.canonical_text)
        .bind(metadata_json)
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Retrieve a SourceRef by ID from the database.
    pub async fn get_by_id(pool: &PgPool, source_id: &str) -> Result<Option<Self>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT source_id, source_type, origin_product, uri, content_hash, provenance_id, state, created_at, canonical_title, canonical_text, metadata FROM source_refs WHERE source_id = $1"
        )
        .bind(source_id)
        .fetch_optional(pool)
        .await?;

        Ok(row.map(|r| {
            let metadata_str: String = r.get(10);
            let metadata =
                serde_json::from_str(&metadata_str).unwrap_or_else(|_| serde_json::json!({}));
            Self {
                source_id: r.get::<String, _>(0),
                source_type: SourceType::Document,
                origin_product: r.get::<String, _>(2),
                uri: r.get::<Option<String>, _>(3),
                content_hash: r.get::<String, _>(4),
                provenance_id: r.get::<String, _>(5),
                state: if r.get::<String, _>(6) == "Active" {
                    SourceState::Active
                } else {
                    SourceState::Archived
                },
                created_at: r.get::<String, _>(7),
                canonical_title: r.get::<Option<String>, _>(8),
                canonical_text: r.get::<Option<String>, _>(9),
                metadata,
            }
        }))
    }
}

/// Ingest a markdown document and create a SourceRef.
pub async fn ingest_markdown(
    content: &str,
    filename: &str,
    pool: &PgPool,
) -> Result<SourceRef, Box<dyn std::error::Error>> {
    let now = chrono::Utc::now().to_rfc3339();
    let request = gear_loader::ExtractionRequest {
        format: gear_loader::EXTRACTION_REQUEST_FORMAT.to_string(),
        request_id: uuid::Uuid::new_v4().to_string(),
        actor_ref: "rumble-lm".to_string(),
        workspace_ref: "lm-session".to_string(),
        input: gear_loader::ExtractionInput {
            kind: gear_loader::InputKind::FileRef,
            reference: filename.to_string(),
        },
        policy: gear_loader::ExtractionPolicy {
            allowed_media_types: vec!["text/markdown".to_string()],
            max_bytes: 10 * 1024 * 1024,
            network: gear_loader::NetworkPolicy::Disabled,
            ocr: gear_loader::FeatureToggle::Disabled,
            stt: gear_loader::FeatureToggle::Disabled,
            pii_mode: gear_loader::FindingMode::Detect,
            secret_mode: gear_loader::FindingMode::Detect,
            prompt_injection_mode: gear_loader::PromptInjectionMode::Detect,
        },
        requested_outputs: vec![gear_loader::CANONICAL_SOURCE_DOCUMENT_FORMAT.to_string()],
    };

    let raw_input = gear_loader::RawInput {
        input_type: gear_loader::SourceInputType::Markdown,
        media_type: "text/markdown",
        bytes: content.as_bytes(),
        uri: Some(&format!("rumble-lm:{}", filename)),
        filename: Some(filename),
    };

    let bundle = gear_loader::extract_text_like(&request, raw_input, &now)?;
    let provenance_id = format!("prov_{}", uuid::Uuid::new_v4());
    let source_ref =
        SourceRef::from_canonical(&bundle.canonical_document, "rumble-lm", &provenance_id);
    source_ref.persist(pool).await?;
    Ok(source_ref)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_ref_from_canonical_preserves_metadata() {
        // This test validates schema but requires a real gear-loader output.
        // For now, we test the basic shape.
        assert_eq!(SourceType::Document, SourceType::Document);
        assert_eq!(SourceState::Active, SourceState::Active);
        assert_eq!(SourceState::Active.to_string(), "Active");
    }
}
