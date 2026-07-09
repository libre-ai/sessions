-- Store ingested source references from gear-loader
CREATE TABLE IF NOT EXISTS source_refs (
    source_id TEXT PRIMARY KEY,
    source_type TEXT NOT NULL,
    origin_product TEXT NOT NULL,
    uri TEXT,
    content_hash TEXT NOT NULL,
    provenance_id TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'Active',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    canonical_title TEXT,
    canonical_text TEXT,
    metadata JSONB DEFAULT '{}'
);

CREATE INDEX idx_source_refs_provenance ON source_refs(provenance_id);
CREATE INDEX idx_source_refs_state ON source_refs(state);

-- Extend questions table to include source references
ALTER TABLE IF EXISTS questions ADD COLUMN source_ref_ids TEXT[] DEFAULT ARRAY[]::TEXT[];

CREATE TABLE IF NOT EXISTS question_source_citations (
    id TEXT PRIMARY KEY,
    question_id TEXT NOT NULL REFERENCES questions(id) ON DELETE CASCADE,
    source_id TEXT NOT NULL REFERENCES source_refs(source_id) ON DELETE RESTRICT,
    citation_index INTEGER NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(question_id, source_id)
);

CREATE INDEX idx_question_source_citations_question ON question_source_citations(question_id);
CREATE INDEX idx_question_source_citations_source ON question_source_citations(source_id);
