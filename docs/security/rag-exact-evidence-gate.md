# RAG exact-evidence gate

- **Status:** implemented prerequisite for issue #33; the notebook vertical itself is not delivered
- **Boundary:** `presto-rag` only; no HTTP endpoint, UI, authorization policy, or provider deployment

## Threat model

Corpus text is untrusted. It may contain instructions, forged prompt markers, or
text intended to make a generator/verifier return `supported=true`. Prompt
instructions and `fenced_source` remain defence in depth, not proof that a model
will ignore that text. The AI provider and its semantic self-verdict are therefore
not sufficient authorities for publishing grounded content.

The authorized `Chunk` supplied by the scoped `Retriever` is the source authority.
For the quiz path, `verify_grounding` accepts a provider `supported=true` only when
all of these deterministic checks also pass:

1. the question cites exactly the authorized source section;
2. the provider supplies a structured source-section id and exact quote;
3. the section id equals the authorized chunk id;
4. the quote is a non-empty byte-exact substring of the chunk;
5. every marked correct answer is a non-empty byte-exact substring of that quote;
6. corpus fence markers are not accepted as evidence.

Only then can the private accepted state of `GroundingVerdict` be constructed and
the pipeline set a public verified citation marker. Missing/mismatched evidence,
`supported=false`, or citation mismatch is unsupported. Malformed/indeterminate
output and provider failure are bounded errors. The pipeline drops all of those
outcomes.

`validate_exact_evidence` and the privately constructed
`ValidatedGroundingEvidence` type live in `presto-rag`, so future notebook
orchestration can reuse the boundary
without depending on `presto-server`. For a notebook answer, the orchestrator must
validate the answer/claims it intends to publish; retrieval or the provider verdict
alone must never construct `RagQueryResponse::Grounded`.

## Deterministic attack proof

`pipeline::tests::source_prompt_injection_cannot_forge_grounded_verdict` runs in
the normal offline Rust suite. Its fake provider follows an instruction embedded
in the source, generates a France/Paris question absent from that source, returns
`supported=true`, and invents evidence. The pipeline still returns no public
question. This test checks the security outcome, not merely prompt-marker presence.
The gated real-provider tests remain supplementary.

## Deliberate limits

- The exact check proves lexical presence only. It does **not** prove semantic
  entailment, truth, relevance, negation handling, or resistance to a poisoned
  source that itself contains a false claim.
- Paraphrases, translations, normalization, and synthesized multi-source answers
  are not accepted by this proof. False rejection is intentional until a stronger
  deterministic scheme is specified and tested.
- The current quiz check binds marked correct answers, not every semantic premise
  implied by arbitrary question wording. The provider verdict remains a semantic
  check, but is never sufficient by itself.
- Source authorization, space isolation, clearance, and source integrity are
  separate upstream controls. Exact evidence does not replace them.
- Clarifications and flashcards are not promoted to a public `Grounded` notebook
  verdict by this gate. Any future public projection must reuse an appropriate
  deterministic evidence check.

Do not log corpus text, prompts, exact quotes, raw provider verdicts/reasons,
tokens, or PII. Operational signals should contain bounded outcome codes only.
