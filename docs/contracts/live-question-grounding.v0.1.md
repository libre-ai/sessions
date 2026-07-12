# live-question-grounding.v0.1 Contract

Participant-facing grounding projection for live-session questions.

## Scope

`QuestionOpened.question.grounding` tells a client whether the server has citation evidence for the current question without exposing corpus text or raw source handles.

This is a runtime projection, not a Gear extraction contract and not a real cross-service Biscuit proof.

## Shape

```json
{
  "grounded": true,
  "citation_count": 1,
  "validation_status": "fixture",
  "source_refs_exposed": false
}
```

Fields:

- `grounded`: `true` only when the server has a non-empty source-section set and an explicit validation marker.
- `citation_count`: count of cited source sections represented by the server-side question.
- `validation_status`:
  - `verified`: RAG path passed the current provider + exact lexical evidence gate after retrieval/generation; this is not proof of truth or complete prompt-injection resistance.
  - `fixture`: deterministic local/demo question; useful for tests, not product-grade provenance.
  - `not_validated`: facilitator-pushed or legacy question with no server-side proof.
- `source_refs_exposed`: always `false` for `question_opened`; clients do not receive source text or raw source-section ids in this message.

## Invariants

1. `QuestionPublic` never includes `correct_choices`.
2. `QuestionPublic` never includes `source_section_ids` or source text.
3. A host-supplied `PushQuestion` is allowed for the wedge, but any client-provided `citation_validation` is stripped and projects as `grounded=false`.
4. The RAG path may set `validation_status=verified` only after `verify_grounding(...)` returns validated exact evidence; a provider boolean alone is insufficient.
5. Fixture validation keeps tests and demos observable without claiming production citation validation.

See `live-question-grounding.v0.1.fixtures.json` for deterministic examples.
