//! Independent final authority for notebook answers.
//!
//! An approved query alias first yields an opaque permit bound to the
//! authenticated space, effective clearance, revision, scoped source hash,
//! canonical answer and citation. Retrieval/provider/verifier output can only
//! produce a candidate; this module alone can compare it with the permit and
//! project the public `Grounded` variant.

use presto_core::api::{ConfidentialityLevel, RagQueryResponse, SourceCitation};
use sha2::{Digest, Sha256};

use crate::notebook_rag::{
    NotebookCandidate, fixture_document_id, fixture_source_section_id, fixture_source_text,
    fixture_title, scoped_source_hash,
};

const SUPPORTED_REVISION: u32 = 1;
const TEMPLATE_CONTROL_HASH: &str =
    "2c7a3e0f000b86c0992ea973c371a545263377e8db1e6b7096353bc799a5582a";
const NO_APPROVED_CLAIM: &str = "no_approved_claim";
const PROVISIONING_POLICY: &str = "personal-space-fixture-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovedClaimsError {
    Unavailable,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ApprovedClaimRegistry {
    unavailable: bool,
}

impl ApprovedClaimRegistry {
    pub const fn fixture() -> Self {
        Self { unavailable: false }
    }

    pub const fn unavailable() -> Self {
        Self { unavailable: true }
    }

    /// Select an approved alias before any untrusted RAG stage executes.
    pub(crate) fn permit(
        &self,
        space_id: &str,
        effective_clearance: ConfidentialityLevel,
        query: &str,
    ) -> Result<Option<ApprovedPermit>, ApprovedClaimsError> {
        if self.unavailable {
            return Err(ApprovedClaimsError::Unavailable);
        }
        Ok(FIXTURE_CLAIMS
            .iter()
            .find_map(|claim| claim.issue_permit(space_id, effective_clearance, query)))
    }

    /// Final authority gate. A provider/source cannot call this with a forged
    /// permit because `ApprovedPermit` has no public constructor or fields.
    pub(crate) fn approve(
        &self,
        permit: ApprovedPermit,
        candidate: NotebookCandidate,
        max_sources: u8,
    ) -> Option<ApprovedAnswer> {
        if self.unavailable || !permit.matches(&candidate) {
            return None;
        }
        Some(ApprovedAnswer::new(permit, max_sources))
    }
}

/// Opaque authorization selected only from an approved alias and bound to one
/// deterministically derived personal-space artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApprovedPermit {
    space_id: String,
    effective_clearance: ConfidentialityLevel,
    claim_id: &'static str,
    revision: u32,
    control_hash: String,
    source_hash: String,
    answer: &'static str,
    citation: SourceCitation,
}

impl ApprovedPermit {
    fn matches(&self, candidate: &NotebookCandidate) -> bool {
        self.revision == SUPPORTED_REVISION
            && self
                .effective_clearance
                .allows(ConfidentialityLevel::Public)
            && self.control_hash == self.computed_control_hash()
            && self.source_hash == candidate.source_hash
            && self.answer == candidate.answer
            && self.citation == candidate.citation
    }

    fn computed_control_hash(&self) -> String {
        hash_fields(&[
            PROVISIONING_POLICY,
            &self.space_id,
            self.claim_id,
            &self.revision.to_string(),
            classification_name(self.effective_clearance),
            &self.source_hash,
            self.answer,
            &self.citation.source_section_id,
            self.citation.document_id.as_deref().unwrap_or_default(),
            self.citation.title.as_deref().unwrap_or_default(),
            self.citation.excerpt.as_deref().unwrap_or_default(),
        ])
    }
}

/// The only server-authority value from which the notebook route projects
/// `Grounded`. Constructor and fields remain private.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApprovedAnswer {
    space_id: String,
    answer: String,
    citations: Vec<SourceCitation>,
}

impl ApprovedAnswer {
    fn new(permit: ApprovedPermit, max_sources: u8) -> Self {
        Self {
            space_id: permit.space_id,
            answer: permit.answer.to_owned(),
            citations: vec![permit.citation]
                .into_iter()
                .take(usize::from(max_sources))
                .collect(),
        }
    }

    pub(crate) fn project_for(self, authenticated_space_id: &str) -> RagQueryResponse {
        if self.space_id != authenticated_space_id {
            return RagQueryResponse::rejected(NO_APPROVED_CLAIM);
        }
        RagQueryResponse::Grounded {
            answer: self.answer,
            citations: self.citations,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ClaimTemplate {
    claim_id: &'static str,
    revision: u32,
    template_control_hash: &'static str,
    provenance: &'static str,
    revoked: bool,
    classification: ConfidentialityLevel,
    aliases: &'static [&'static str],
    answer: &'static str,
}

impl ClaimTemplate {
    fn issue_permit(
        &self,
        space_id: &str,
        effective_clearance: ConfidentialityLevel,
        normalized_query: &str,
    ) -> Option<ApprovedPermit> {
        if space_id.is_empty()
            || self.revoked
            || self.revision != SUPPORTED_REVISION
            || !effective_clearance.allows(self.classification)
            || self.template_control_hash != self.computed_template_hash()
            || !self.aliases.contains(&normalized_query)
        {
            return None;
        }
        let source_section_id = fixture_source_section_id(space_id);
        let document_id = fixture_document_id(space_id);
        let source_hash = scoped_source_hash(space_id, &source_section_id, fixture_source_text());
        let citation = SourceCitation {
            source_section_id,
            document_id: Some(document_id),
            title: Some(fixture_title().to_owned()),
            excerpt: Some(fixture_source_text().to_owned()),
        };
        let mut permit = ApprovedPermit {
            space_id: space_id.to_owned(),
            effective_clearance,
            claim_id: self.claim_id,
            revision: self.revision,
            control_hash: String::new(),
            source_hash,
            answer: self.answer,
            citation,
        };
        permit.control_hash = permit.computed_control_hash();
        Some(permit)
    }

    fn computed_template_hash(&self) -> String {
        let mut fields = vec![
            PROVISIONING_POLICY,
            self.claim_id,
            self.provenance,
            if self.revoked { "revoked" } else { "active" },
            classification_name(self.classification),
            self.answer,
            fixture_source_text(),
            fixture_title(),
        ];
        let revision = self.revision.to_string();
        fields.insert(2, &revision);
        fields.extend_from_slice(self.aliases);
        hash_fields(&fields)
    }
}

const FIXTURE_ALIASES: &[&str] = &[
    "quelle est la capitale de la france ?",
    "quelle est la capitale de la france?",
    "capitale de la france",
    "what is the capital of france?",
];

const FIXTURE_CLAIMS: &[ClaimTemplate] = &[ClaimTemplate {
    claim_id: "approved-capital-france-v1",
    revision: SUPPORTED_REVISION,
    template_control_hash: TEMPLATE_CONTROL_HASH,
    provenance: "control://fixtures/approved-geography/v1",
    revoked: false,
    classification: ConfidentialityLevel::Public,
    aliases: FIXTURE_ALIASES,
    answer: "Paris est la capitale de la France.",
}];

pub(crate) fn normalize_query(query: &str) -> String {
    query
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
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

const fn classification_name(level: ConfidentialityLevel) -> &'static str {
    match level {
        ConfidentialityLevel::Public => "public",
        ConfidentialityLevel::Internal => "internal",
        ConfidentialityLevel::Confidential => "confidential",
        ConfidentialityLevel::Secret => "secret",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matching_candidate(permit: &ApprovedPermit) -> NotebookCandidate {
        NotebookCandidate {
            answer: permit.answer.to_owned(),
            citation: permit.citation.clone(),
            source_hash: permit.source_hash.clone(),
        }
    }

    #[test]
    fn template_hash_is_valid_and_clearance_is_checked_independently() {
        let public = ClaimTemplate {
            classification: ConfidentialityLevel::Internal,
            template_control_hash: "invalid until recomputed",
            ..FIXTURE_CLAIMS[0]
        };
        let valid_hash = Box::leak(public.computed_template_hash().into_boxed_str());
        let internal = ClaimTemplate {
            template_control_hash: valid_hash,
            ..public
        };
        assert_eq!(
            FIXTURE_CLAIMS[0].computed_template_hash(),
            TEMPLATE_CONTROL_HASH
        );
        assert!(
            internal
                .issue_permit("space-a", ConfidentialityLevel::Public, FIXTURE_ALIASES[2])
                .is_none()
        );
        assert!(
            internal
                .issue_permit(
                    "space-a",
                    ConfidentialityLevel::Internal,
                    FIXTURE_ALIASES[2]
                )
                .is_some()
        );
    }

    #[test]
    fn derived_spaces_have_distinct_source_ids_hashes_and_controls() {
        let registry = ApprovedClaimRegistry::fixture();
        let permit_a = registry
            .permit("space-a", ConfidentialityLevel::Public, FIXTURE_ALIASES[2])
            .unwrap()
            .unwrap();
        let permit_b = registry
            .permit("space-b", ConfidentialityLevel::Public, FIXTURE_ALIASES[2])
            .unwrap()
            .unwrap();
        assert_ne!(
            permit_a.citation.source_section_id,
            permit_b.citation.source_section_id
        );
        assert_ne!(permit_a.source_hash, permit_b.source_hash);
        assert_ne!(permit_a.control_hash, permit_b.control_hash);

        let candidate_a = matching_candidate(&permit_a);
        assert!(registry.approve(permit_b, candidate_a, 1).is_none());
    }

    #[test]
    fn final_authority_rejects_answer_source_hash_and_citation_tampering() {
        let registry = ApprovedClaimRegistry::fixture();
        for mutation in 0..3 {
            let permit = registry
                .permit(
                    "space-a",
                    ConfidentialityLevel::Internal,
                    FIXTURE_ALIASES[2],
                )
                .unwrap()
                .unwrap();
            let mut candidate = matching_candidate(&permit);
            match mutation {
                0 => candidate.answer = "Paris".into(),
                1 => candidate.source_hash = "forged".into(),
                _ => candidate.citation.source_section_id = "foreign#source".into(),
            }
            assert!(registry.approve(permit, candidate, 1).is_none());
        }
    }

    #[test]
    fn approved_answer_rechecks_space_at_projection() {
        let registry = ApprovedClaimRegistry::fixture();
        let permit = registry
            .permit("space-a", ConfidentialityLevel::Public, FIXTURE_ALIASES[2])
            .unwrap()
            .unwrap();
        let answer = registry
            .approve(permit.clone(), matching_candidate(&permit), 1)
            .unwrap();
        assert_eq!(
            answer.project_for("space-b"),
            RagQueryResponse::rejected(NO_APPROVED_CLAIM)
        );
    }
}
