[English](README.md) · **Français**

> [!NOTE]
> **Réservé · futur foyer de Sessions** — reconstruit dans le dépôt de base canonique [`libre-ai/libre-ai`](https://github.com/libre-ai/libre-ai) ([topologie multi-dépôts, ADR-0008](https://github.com/libre-ai/libre-ai/blob/main/docs/adr/0008-multi-repo-target-topology-and-brand.md)).
> Ce dépôt rouvrira comme dépôt produit réel lorsque le propriétaire l'activera, consommant la base comme dépendance versionnée. Les fondations décrites ci-dessous sont **en cours de construction** — avec des liens vers le code qui existe déjà.

# Sessions

**Apprentissage collectif sourcé et facilitation en temps réel.** Réunissez un groupe autour de matériaux sourcés — articles, preuves, contribution d'experts — avec des rôles explicites (facilitateur, participant, observateur), des règles d'audience pour chaque contribution, et une **porte d'approbation humaine** avant que tout résultat partagé soit publié. Jamais une synthèse silencieuse ; jamais une export qui révèle par défaut des contributions privées.

Le cas canonique auquel il répond : _« Comment exécuter une session d'apprentissage collectif sourcée en temps réel, où chaque résultat est traçable et l'approbation est obligatoire ? »_ — sur des données que les participants possèdent, dans un espace où la preuve est citée, et où un facilitateur peut révoquer les sources ou suspendre la session sans perdre l'historique.

## Ce qui le distingue

- **Approbation avant publication.** Les facilitateurs demandent une synthèse à partir de sources bornées et de contributions des participants ; le résultat reste brouillon jusqu'à ce qu'un humain l'approuve explicitement. La génération est une aide, pas l'autorité.
- **Scopes d'audience par défaut.** Chaque contribution porte une politique d'audience (publique, partagée avec le groupe, privée). Une export n'inclut que le contenu que le demandeur est autorisé à voir ; les contributions privées ne fuient jamais silencieusement dans les résultats partagés.
- **Immuable et auditable.** Tous les événements — participants rejoignant, contributions envoyées, synthèses approuvées — sont immuables et protégés par sécurité au niveau des lignes par organisation. La révocation bloque les futures synthèses mais ne réécrit jamais la preuve passée.
- **Sourcé et borné.** Une synthèse ne peut référencer que les sources explicitement jointes et validées par le facilitateur. La récupération RAG n'est pas l'autorité ; les sources jointes le sont.
- **Collaboration en temps réel, résiliente.** Les participants co-éditent les brouillons de résultats en temps réel lorsqu'un relais auto-hébergé est disponible ; la session se dégrade gracieusement en mode append-only si le relais est inatteignable, sans jamais perdre de données.
- **Accès bloqué par défaut.** Un participant inconnu, un rôle manquant dans la session, ou un curseur obsolète est rejeté immédiatement. Jamais dégradé silencieusement.

## État — spécifié publiquement, fondations en construction

Sessions est en construction à partir d'une spécification verrouillée. Il **n'est pas encore publié** ; la persistence append-only et l'autorisation viennent d'abord, et une bonne partie existe déjà et est prouvée dans le dépôt de base :

| Fondation                                                     | État                    | Preuve                                                                                                                                                                                                           |
| ------------------------------------------------------------- | ----------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Validateur d'événement session & réducteur append-only**    | ✅ construit            | Transitions d'état testées unitairement et idempotence ([#165](https://github.com/libre-ai/libre-ai/pull/165))                                                                                                   |
| **Persistence d'événement append-only avec RLS**              | ✅ construit, intégré   | Sécurité au niveau des lignes PostgreSQL, isolation par locataire, reconnexion par curseur ([#173](https://github.com/libre-ai/libre-ai/pull/173))                                                               |
| **Matrice d'autorisation & politique Biscuit**                | ✅ construit, conforme  | Rôles d'adhésion, scope des ressources (session/contribution/résultat), révocation ([#174](https://github.com/libre-ai/libre-ai/pull/174))                                                                       |
| **Service de commandes & composition verticale**              | ✅ construit            | Commandes métier (CreateSession, JoinSession, SubmitContribution, ApproveOutcome), en direct ([#175](https://github.com/libre-ai/libre-ai/pull/175))                                                             |
| **Cockpit accessible SSR — vue de lecture**                   | ✅ construit, HTTP prêt | Navigation au clavier, lecture d'état de session, interrogation du flux d'événement ([#179](https://github.com/libre-ai/libre-ai/pull/179))                                                                      |
| **Amendements collaboration temps réel — spec**               | ✅ spec-signé           | Design collab ratifié par propriétaire : CRDT + MLS E2EE, relais auto-hébergé, événements CollabCheckpointRecorded, porte d'approbation jamais affaiblie ([#198](https://github.com/libre-ai/libre-ai/pull/198)) |
| **Brique collaboration (CRDT + MLS) — implémentation**        | ⏳ suite                | Co-édition souveraine chiffrée de bout en bout ; relais chiffré uniquement ; dégradation gracieuse en append-only                                                                                                |
| **Surface de commandes — UI d'écriture, export, suppression** | ⏳ suite                | Workflows brouillon/approbation, export scopes d'audience, rétention et fermeture de session                                                                                                                     |
| **Adaptateur génération & preuve**                            | ⏳ suite                | Synthèse bornée à partir de sources/contributions, Biscuit atténué pour fournisseur, gestion d'échecs de brouillon                                                                                               |
| **Qualification multi-instance & confidentialité**            | ⏳ suite                | Reconnexion deux-instances, preuve d'export privée, déni cross-tenant, parcours d'approbation humain                                                                                                             |

Ce dépôt est `private` jusqu'à ce que le propriétaire l'active pour ouverture publique (vague 4). **Cible de référence :** Miro — outillage de facilitation collaborative en temps réel, atteint par approbation explicite et événements append-only plutôt que consensus temps réel.

## Comment ça fonctionne

1. **Faciliter** — un facilitateur crée une session, définit une politique d'audience pour les contributions (publique, partagée, privée), et joint des sources validées (documents, réponses d'experts, preuves antérieures).
2. **Participer** — les participants rejoignent avec adhésion scopes, contribuent selon les règles d'audience, et se reconnectent à partir d'un curseur sans re-soumettre les contributions. La présence est éphémère et ne peut pas autoriser.
3. **Synthèse** — le facilitateur demande une synthèse à partir de sources et contributions en scope ; le résultat est rédigé par un fournisseur de génération, mais reste **brouillon uniquement** jusqu'à ce que le facilitateur l'approuve explicitement.
4. **Export & fermeture** — un acteur autorisé exporte un paquet scopes d'audience (ne voyant que le contenu auquel il peut accéder), et le propriétaire ferme ou supprime la session selon le contrat de rétention. Tous les événements restent immuables.

## Architecture — assemblée à partir de briques interopérables

Sessions est un produit assemblé à partir de briques versionnées indépendamment ; chacune est utilisable et testable seule, et le produit est leur composition (la cible multi-dépôts de [l'ADR-0008](https://github.com/libre-ai/libre-ai/blob/main/docs/adr/0008-multi-repo-target-topology-and-brand.md)).

| Brique                                    | Rôle                                                 | Interface exposée / consommée                                                                                                            |
| ----------------------------------------- | ---------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| **`sessions-core`** (TypeScript / Bun)    | Machine d'état déterministe et réducteur d'événement | Commandes métier, flux d'événement append-only, projection d'audience, logique de curseur reconnexion                                    |
| **`@libre-ai/web-platform`**              | Fondation SSR / BFF Bun                              | Gestionnaire de requêtes, passage en WebSocket, évaluation session côté serveur, interface requête base de données                       |
| **`@libre-ai/data`**                      | Persistence organisation, session et contribution    | Pilote PostgreSQL, politiques sécurité au niveau des lignes, framework migration                                                         |
| **`collab-core`** (CRDT + MLS, Rust/WASM) | Co-édition collaborative de brouillon temps réel     | Synchronisation participant E2EE, interface relais chiffré uniquement, intégration événement CollabCheckpointRecorded (⏳ prévue)        |
| **Contrats**                              | Surface d'interopérabilité verrouillée               | Schémas `session-event.v1`, `session-export.v1`, `evidence-report.v1`, OpenAPI `sessions.v1.yaml`, politique authz `sessions-v1.datalog` |

L'hôte (serveur Bun) détient le jeton d'autorisation et évalue les commandes contre la politique Biscuit ; le réducteur d'événement s'exécute déterministe localement ; la collab temps réel est isolée par capacité (le relais reçoit uniquement du chiffré, jamais de clés cryptographiques).

## Où se déroule le travail

Tout le développement actif est dans le dépôt de base, sous :

- `apps/sessions` — l'hôte produit (cockpit SSR, persistence d'événement, service de commandes, UI)
- `src/domain` — machine d'état, définitions d'événement, logique d'audience
- `src/persistence` — PostgreSQL RLS, reconnexion, curseur d'événement
- `src/authz` — politique d'autorisation Biscuit, validation de rôle
- `src/server` — gestionnaires de commandes WebSocket et HTTP
- `src/ui` — cockpit accessible lecture/écriture (React 19)
- `contracts/` — schémas verrouillés événement session, export et API
- [`docs/apps/sessions.md`](https://github.com/libre-ai/libre-ai/blob/main/docs/apps/sessions.md) — la spécification produit complète

Pour suivre l'avancement ou contribuer, ouvrez issues et pull requests dans [`libre-ai/libre-ai`](https://github.com/libre-ai/libre-ai). Ce dépôt reste réservé jusqu'à son activation.

## Licence

EUPL-1.2.
