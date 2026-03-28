//! Corpus ingestion and retrieval over Postgres + pgvector.
//!
//! Text is split into chunks, embedded through the [`AiProvider`] seam, and
//! stored with a `vector` column; retrieval embeds the query and ranks chunks by
//! cosine distance. Embeddings are passed as `[...]::vector` literals, so no
//! extra binding crate is required.
//!
//! # Security: the corpus is trusted
//!
//! Ingested text flows verbatim into the generation and grounding-verifier
//! prompts ([`crate::generate`], [`crate::verify`]); [`CorpusStore::ingest`] does
//! NOT sanitize it for prompt injection. The corpus is therefore assumed to come
//! from a trusted (admin) source. When user-supplied ingestion is added, that
//! text must be treated as untrusted and isolated from prompt construction.

use async_trait::async_trait;
use sqlx::Row;
use sqlx::postgres::PgPool;

use crate::provider::{AiError, AiProvider};

/// A unit of source text with a stable section id (for grounding citations).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub source_section_id: String,
    pub text: String,
}

/// A retrieved chunk with its cosine distance to the query (smaller is closer).
#[derive(Debug, Clone)]
pub struct Retrieved {
    pub source_section_id: String,
    pub text: String,
    pub distance: f32,
}

/// An ingestion/retrieval failure.
#[derive(Debug)]
pub struct CorpusError(pub String);

impl std::fmt::Display for CorpusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "corpus error: {}", self.0)
    }
}

impl std::error::Error for CorpusError {}

impl From<sqlx::Error> for CorpusError {
    fn from(e: sqlx::Error) -> Self {
        CorpusError(e.to_string())
    }
}

impl From<AiError> for CorpusError {
    fn from(e: AiError) -> Self {
        CorpusError(e.to_string())
    }
}

/// Split a document into chunks (one per paragraph), each with a section id of
/// the form `{document_id}#p{ordinal}`.
pub fn chunk(document_id: &str, text: &str) -> Vec<Chunk> {
    text.split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .enumerate()
        .map(|(i, p)| Chunk {
            source_section_id: format!("{document_id}#p{i}"),
            text: p.to_string(),
        })
        .collect()
}

fn vector_literal(v: &[f32]) -> String {
    let mut out = String::from("[");
    for (i, x) in v.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&x.to_string());
    }
    out.push(']');
    out
}

/// Ranked retrieval over a corpus, abstracted so the generation pipeline can be
/// tested without a database.
#[async_trait]
pub trait Retriever: Send + Sync {
    /// Retrieve the `k` chunks closest to `query`.
    async fn retrieve(
        &self,
        query: &str,
        k: usize,
        provider: &dyn AiProvider,
    ) -> Result<Vec<Retrieved>, CorpusError>;

    /// Fetch a chunk by its exact source-section id (for grounded breakouts).
    async fn fetch_section(&self, section_id: &str) -> Result<Option<Chunk>, CorpusError>;
}

/// Corpus storage in Postgres + pgvector.
pub struct CorpusStore {
    pool: PgPool,
}

impl CorpusStore {
    /// Connect, ensure the `vector` extension, and create the chunks table. The
    /// embedding column is dimension-free (pgvector enforces dimension per value),
    /// so one provider's embeddings define the corpus dimension at insert time.
    pub async fn connect(url: &str) -> Result<Self, CorpusError> {
        let pool = PgPool::connect(url).await?;
        sqlx::raw_sql(
            "CREATE EXTENSION IF NOT EXISTS vector; \
             CREATE TABLE IF NOT EXISTS presto_chunks (\
                document_id       TEXT NOT NULL, \
                ordinal           INT NOT NULL, \
                source_section_id TEXT NOT NULL, \
                text              TEXT NOT NULL, \
                embedding         vector NOT NULL, \
                PRIMARY KEY (document_id, ordinal));",
        )
        .execute(&pool)
        .await?;
        Ok(Self { pool })
    }

    /// Ingest a document: chunk, embed, and replace any prior chunks for it.
    /// Returns the number of chunks stored.
    pub async fn ingest(
        &self,
        document_id: &str,
        text: &str,
        provider: &dyn AiProvider,
    ) -> Result<usize, CorpusError> {
        let chunks = chunk(document_id, text);
        if chunks.is_empty() {
            return Ok(0);
        }
        let texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
        let embeddings = provider.embed(&texts).await?;

        sqlx::query("DELETE FROM presto_chunks WHERE document_id = $1")
            .bind(document_id)
            .execute(&self.pool)
            .await?;
        for (i, (c, embedding)) in chunks.iter().zip(embeddings).enumerate() {
            sqlx::query(
                "INSERT INTO presto_chunks (document_id, ordinal, source_section_id, text, embedding) \
                 VALUES ($1, $2, $3, $4, $5::vector)",
            )
            .bind(document_id)
            .bind(i as i32)
            .bind(&c.source_section_id)
            .bind(&c.text)
            .bind(vector_literal(&embedding))
            .execute(&self.pool)
            .await?;
        }
        Ok(chunks.len())
    }
}

#[async_trait]
impl Retriever for CorpusStore {
    async fn retrieve(
        &self,
        query: &str,
        k: usize,
        provider: &dyn AiProvider,
    ) -> Result<Vec<Retrieved>, CorpusError> {
        let embedding = provider
            .embed(&[query.to_string()])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| CorpusError("provider returned no embedding".into()))?;
        let literal = vector_literal(&embedding);
        let rows = sqlx::query(
            "SELECT source_section_id, text, (embedding <=> $1::vector) AS distance \
             FROM presto_chunks ORDER BY embedding <=> $1::vector LIMIT $2",
        )
        .bind(literal)
        .bind(k as i64)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|r| Retrieved {
                source_section_id: r.get("source_section_id"),
                text: r.get("text"),
                distance: r.get::<f64, _>("distance") as f32,
            })
            .collect())
    }

    async fn fetch_section(&self, section_id: &str) -> Result<Option<Chunk>, CorpusError> {
        let row = sqlx::query(
            "SELECT source_section_id, text FROM presto_chunks \
             WHERE source_section_id = $1 LIMIT 1",
        )
        .bind(section_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| Chunk {
            source_section_id: r.get("source_section_id"),
            text: r.get("text"),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_splits_paragraphs_with_section_ids() {
        let chunks = chunk("doc1", "First para.\n\nSecond para.\n\n\n  Third  ");
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].source_section_id, "doc1#p0");
        assert_eq!(chunks[1].text, "Second para.");
        assert_eq!(chunks[2].source_section_id, "doc1#p2");
        assert_eq!(chunks[2].text, "Third");
    }

    #[test]
    fn vector_literal_formats_for_pgvector() {
        assert_eq!(vector_literal(&[1.0, 2.5, -3.0]), "[1,2.5,-3]");
    }
}
