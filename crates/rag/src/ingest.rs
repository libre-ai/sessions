//! Turning an uploaded document body into corpus-ready text.
//!
//! Text and Markdown are taken as UTF-8 and handed straight to the corpus
//! chunker. Binary formats (PDF, DOCX) are deliberately **not** parsed here yet:
//! a parser running on untrusted upload bytes is an attack surface (malformed
//! input can panic or be adversarial), so those land in a later increment behind
//! `catch_unwind` isolation rather than being bolted on now.

/// A document-parsing failure (unsupported type, bad encoding, empty body).
#[derive(Debug, PartialEq, Eq)]
pub struct IngestError(pub String);

impl std::fmt::Display for IngestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ingest error: {}", self.0)
    }
}

impl std::error::Error for IngestError {}

/// Content types accepted for ingestion.
const SUPPORTED: &[&str] = &["text/plain", "text/markdown", "text/x-markdown"];

/// Extract corpus-ready text from an uploaded body, dispatching on its content
/// type. Only UTF-8 text and Markdown are supported today.
pub fn document_text(content_type: &str, bytes: &[u8]) -> Result<String, IngestError> {
    // Normalize `text/markdown; charset=utf-8` → `text/markdown`.
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if !SUPPORTED.contains(&mime.as_str()) {
        return Err(IngestError(format!(
            "unsupported content type '{mime}'; supported: text/plain, text/markdown"
        )));
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| IngestError("document body is not valid UTF-8".into()))?;
    if text.trim().is_empty() {
        return Err(IngestError("document is empty".into()));
    }
    Ok(text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_text_and_markdown_with_or_without_charset() {
        assert_eq!(document_text("text/plain", b"hello").unwrap(), "hello");
        assert_eq!(
            document_text("text/markdown; charset=utf-8", b"# Title\n\nBody").unwrap(),
            "# Title\n\nBody"
        );
        // Case- and whitespace-insensitive on the mime.
        assert!(document_text("  TEXT/Markdown ", b"x").is_ok());
    }

    #[test]
    fn rejects_unsupported_type() {
        let err = document_text("application/pdf", b"%PDF-1.7").unwrap_err();
        assert!(err.0.contains("unsupported content type 'application/pdf'"));
    }

    #[test]
    fn rejects_invalid_utf8() {
        // Lone continuation byte: not valid UTF-8.
        assert_eq!(
            document_text("text/plain", &[0x80]).unwrap_err(),
            IngestError("document body is not valid UTF-8".into())
        );
    }

    #[test]
    fn rejects_empty_or_whitespace() {
        assert!(document_text("text/plain", b"   \n\t ").is_err());
    }
}
