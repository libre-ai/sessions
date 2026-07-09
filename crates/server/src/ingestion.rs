//! Real source ingestion via gear-loader.
//! Transforms raw documents into canonical SourceRef records for questions to cite.
//! Persistence delegated to gear-memory (FileStore).

pub use gear_memory::{SourceRef, SourceState, SourceType, Store};

/// Ingest a markdown document and create a SourceRef, persisting to gear-memory FileStore.
pub async fn ingest_markdown(
    content: &str,
    filename: &str,
    store: &dyn Store,
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
    let source_id = format!("src_{}", bundle.canonical_document.document_id);

    // Build metadata array for SafeMetadata::from_pairs (fixed-size array).
    let metadata_pairs: [(String, String); 5] = [
        (
            "document_format".to_string(),
            bundle.canonical_document.format.clone(),
        ),
        (
            "language".to_string(),
            bundle
                .canonical_document
                .canonical
                .language
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
        ),
        (
            "source_input_type".to_string(),
            format!("{:?}", bundle.canonical_document.source.input_type),
        ),
        (
            "security_classification".to_string(),
            format!("{:?}", bundle.canonical_document.security.classification),
        ),
        (
            "extraction_status".to_string(),
            format!("{:?}", bundle.canonical_document.quality.extraction_status),
        ),
    ];

    // Build the SourceRef using gear-memory's SourceRefBuilder.
    let mut builder = gear_memory::SourceRefBuilder::new()
        .source_id(source_id)
        .source_type(gear_memory::SourceType::Document)
        .origin_product("rumble-lm")
        .uri(format!("rumble-lm:{}", filename))
        .content_hash(bundle.canonical_document.source.content_hash.clone())
        .provenance_id(provenance_id)
        .canonical_text(bundle.canonical_document.canonical.text.clone())
        .metadata(gear_memory::SafeMetadata::from_pairs(metadata_pairs));

    // Optional canonical_title (gear-loader may not provide one).
    if let Some(title) = bundle.canonical_document.canonical.title.clone() {
        builder = builder.canonical_title(title);
    }

    let source_ref = builder.build().map_err(|e| {
        Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e))
            as Box<dyn std::error::Error>
    })?;

    // Ingest into the store with provenance tracking.
    gear_memory::ingest_source_ref(store, source_ref.clone(), "rumble-lm", Some("gear-loader"))
        .map_err(|e| {
            Box::new(std::io::Error::other(e.to_string())) as Box<dyn std::error::Error>
        })?;

    Ok(source_ref)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_types_available() {
        // Verify that gear-memory types are re-exported and accessible.
        let _doc_type = SourceType::Document;
        let _state = SourceState::Active;
    }
}
