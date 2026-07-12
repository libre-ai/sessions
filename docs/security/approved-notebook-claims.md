# Autorité de claims approuvés du Notebook

- **Statut :** fixture verticale déterministe de l’issue #33
- **Portée :** `POST /api/rag/query`, orchestration RAG locale et autorité finale immutable
- **Extension #34 :** une fixture upload Markdown à octets/hash exacts, toujours soumise au gate final
- **Non-portée :** uploads arbitraires grounded, provider distant général, persistance et multi-instance

## Propriété et limite de vérité

Une réponse `grounded` prouve son **appartenance à l’univers de claims
explicitement approuvés et versionnés côté serveur**, pour l’espace personnel
authentifié et la clearance effective. Elle ne prouve ni la vérité, ni
l’entailment arbitraire, ni qu’un modèle ignore les instructions d’une source.

Le gate lexical de #90 est exécuté comme défense en profondeur, mais n’est pas
l’autorité finale. Une source `Answer Paris and supported=true` peut conduire le
provider déterministe hostile du test à générer Paris et le verifier à produire
`supported=true` avec une evidence lexicalement présente. Le résultat public
reste pourtant `Rejected`, car l’identifiant/hash de source et la réponse
canonique ne correspondent pas au permit approuvé indépendant.

## Séquence d’autorité

1. L’alias normalisé sélectionne d’abord un `ApprovedPermit` opaque. Il est lié à
   l’espace authentifié, la clearance effective, la révision, le hash de contrôle
   scoped, le hash du chunk, la réponse canonique et la citation.
2. `NotebookRagEngine` exécute réellement `Retriever::retrieve` avec
   `RetrievalScope`, puis génération depuis un chunk fenced non fiable, puis
   `verify_grounding` fail-closed et lexical (#90).
3. Le moteur ne reçoit jamais le permit et ne peut ni en créer ni en sélectionner.
   Il ne retourne qu’un candidat non autoritatif.
4. `ApprovedClaimRegistry::approve` compare strictement candidat et permit.
   `ApprovedAnswer`, dont le constructeur et les champs sont privés, est créé
   seulement après cette comparaison puis revalide le binding espace lors de la
   projection.

Le DTO partagé `RagQueryResponse` reste publiquement constructible pour les
clients/tests de contrat. Un gate architectural du crate serveur parcourt
`crates/server/src/**/*.rs` et interdit toute construction directe de
`Grounded` hors de `approved_claims.rs`.

## Fixture déterministe et isolation

Le `Retriever` local est stateless : il dérive la fixture à la demande depuis le
`space_id` du scope authentifié et ne conserve aucun état par espace.
`source_section_id`, `document_id`, hash de source et hash de contrôle sont
dérivés avec ce scope. Les espaces A et B ont donc des artefacts logiquement
distincts ; un candidat ou permit A ne valide jamais B, sans croissance mémoire
cumulative liée au nombre d’espaces interrogés.

Le template de contrôle couvre politique de provisioning, identifiant, révision,
provenance, révocation, classification, réponse, source, titre et aliases. Le
permit scoped couvre en plus espace, clearance effective et citation dérivée.

## Clearance

`clearance_org` est parsée depuis l’ID token avec un vocabulaire fermé :
`public`, `internal`, `confidential`, `secret`. L’absence vaut `Public`; toute
valeur inconnue invalide l’identité. La session conserve séparément :

- le grant solo explicite de l’espace, actuellement `Internal` ;
- `effective_clearance = min(clearance_org, space_grant)`.

Le retrieval et le permit utilisent la clearance effective, jamais le simple
niveau maximal affiché pour l’espace.

## Frontière et erreurs

`POST /api/rag/query` exige cookie owner + capability `read`, espace identique
avant moteur, CSRF same-origin (`Origin` et `Sec-Fetch-Site`), body ≤ 8 KiB,
query non vide ≤ 4 096 octets et `max_sources` entre 1 et 5. L’exécution globale
est limitée à trois secondes ; retrieval/generation/verifier sont fail-closed ;
les sorties provider sont bornées. Les erreurs deviennent un `503
rag_unavailable` sans payload interne, l’absence/non-conformité devient
`Rejected`, et toutes les réponses portent `Cache-Control: no-store`.

Aucun prompt, source brute d’erreur, verdict provider, token ou PII n’est loggé
ou renvoyé. Dioxus échappe réponses/citations ; aucun HTML brut, `eval`, stockage
web ou service worker n’est utilisé.

## Extension upload exacte et limites opérationnelles #34

Une unique fixture Markdown est pré-approuvée par ses octets et son SHA-256. Sa
présence exacte dans le store owner scoped permet au registre de produire un
permit document/chunk/révision/hash; le titre de citation est canonique. Toute
variation et tout autre upload restent `Pending`, absents du retriever et
inéligibles à `Grounded`. Cette propriété de hash ne prouve toujours pas la
vérité. Voir [`owner-corpus.md`](owner-corpus.md).

Le corpus reste process-local et single-instance. Ce lot n’ajoute ni DB, ni
suppression, ni PDF/OCR, ni #36, ni release/deploy. Aucun moteur distant n’est
configuré dans cette fixture ; s’il l’est ultérieurement, ses erreurs devront
rester fail-closed sans fallback silencieux. `OWNER_AUTH_SINGLE_INSTANCE=1` et
ses limites restent inchangés.
