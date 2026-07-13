# Traversée locale de la pile — Sessions — 2026-07-13

## Verdict borné

La chaîne technique locale repasse avec les contrats actuels : inspection UI Proof Kit, inspection PostgreSQL statique, manifeste d’artefact, autorisation Biscuit et planification Agent Factory. Cette preuve ne démontre ni disponibilité publique, ni session utilisateur complète, ni exploitation de production.

## Résultats

| Maillon | Résultat | Preuve |
| --- | --- | --- |
| Proof Kit UI | `passed`, 7 contrôles, 0 finding | `proof-kit-ui.json` |
| Proof Kit DB Inspect | `passed`, 0 finding, RLS forcée sur les 2 tables | `db-inspection.json` |
| Artifact Supply | manifeste produit pour `presto-server`, SHA-256 interne `9dbc1f2a…16acc9` | `artifact-manifest.json` |
| Agent Factory | 0 finding, 12 gates passées, 3 tâches planning-only | `agent-factory-dry-run.json` |

Hashes SHA-256 des rapports versionnés :

```text
b0ffea37177b39235c496d8a69bae45406519f72371c0c531bace7468f812719  agent-factory-dry-run.json
fba2f61e6698a97c7d02fce3eb5220aba8f414d67260f2ea37c48de3892c2d9b  artifact-manifest.json
6f51034220795732a8873af11494d27b9a67b00f5f8db3df0589f16a0e0093a0  db-inspection.json
23cd0443a647c80cbffab068c328d75ca7e294c2d7af52fe8622a6456984eaf9  proof-kit-ui.json
```

## Refus initial et corrections

Le premier rejeu a échoué, correctement, sur quatre écarts :

1. `source.organization_id` absent du handoff ;
2. applicabilité de l’inspection DB absente ;
3. bloc PostgreSQL `DO` soumis à revue procédurale et donc bloqué par DB Inspect ;
4. autorisation Biscuit obligatoire mais matériel absent.

Corrections appliquées :

- organisation `libre-ai` déclarée dans le handoff ;
- DB PostgreSQL déclarée avec rapport, profil, version et hash liés ;
- politiques RLS rendues déclaratives dans la migration SQLx versionnée, sans waiver ;
- jeton Biscuit local de dix minutes, clé privée uniquement en mémoire, répertoire éphémère mode `0700`, bearer créé directement en `0600`, suppression du répertoire après le rejeu ;
- branches RSA vulnérables retirées des graphes Agent Factory et Sessions au profit du backend AWS-LC local, sans service AWS ni transfert de données.

Le refus initial est une preuve utile : la chaîne ne transforme pas automatiquement un ancien handoff vert en nouveau handoff accepté lorsque les gates évoluent.

## Reproduction

```bash
# Depuis sessions/, avec proof-kit et agent-factory comme dépôts frères.
./scripts/generate-stack-proof.sh
```

Le script :

1. inspecte `sessions/crates/ui` avec Proof Kit ;
2. inspecte la migration jobs/outbox avec DB Inspect ;
3. construit le bundle propriétaire et `presto-server`, puis émet son manifeste ;
4. génère du matériel Biscuit éphémère local ;
5. exécute `handoff plan --dry-run` avec les preuves UI et DB ;
6. refuse tout rapport non vert et supprime le matériel d’autorisation.

## Limites et travaux restant

- l’inspection DB est statique ; le test PostgreSQL avec rôle non-superuser reste requis ;
- le manifeste atteste le binaire local, pas une release signée, distribuée, reproductible ou restaurée ;
- le manifeste de release est généré séparément : l’option Agent Factory `--evidence-manifest` accepte actuellement des rapports d’inspection, pas un `release_asset`; le gate `artifact_supply_chain_verified` reste donc un squelette dans ce dry-run ;
- le gate `sovereignty_and_license_audit` indique également une intégration externe squelette dans le détail Agent Factory ;
- l’audit conserve uniquement l’acceptation temporaire `RUSTSEC-2026-0173` (`proc-macro-error2`, transitif build-time via Biscuit) ; les branches `RUSTSEC-2023-0071` ont été retirées des deux graphes verrouillés ;
- aucun fournisseur IA réel, aucune donnée personnelle et aucun secret de production n’ont été utilisés ;
- la preuve Biscuit est éphémère et locale ; elle ne remplace pas un service d’identité ou une rotation de clés de production ;
- la CI Sessions doit adopter une révision Agent Factory contenant le générateur de fixture avant que ce rejeu devienne un gate distant ;
- Website ne doit republier ce rapport qu’après revue humaine, mise à jour des noms publics et rattachement à la route de correction.
