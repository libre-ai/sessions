# Liens de join live — sécurité et migration

- **Statut :** tranche de migration pour l’issue #35
- **Portée :** `POST /sessions` (ajout `secure_join_url`), `POST /join/{session_id}/participants`
- **Non-portée :** UI/bundle join, suppression du legacy, preuve de fin de #35

## Sécurité

Le lien sécurisé transporte le Biscuit dans le **fragment** :
`/join/{CODE}#token=<...>`. Le fragment ne part jamais au serveur, ne doit pas
être loggé, et ne doit pas être recopié dans une query string. Le serveur garde
la forme legacy `join_url=/?s=CODE` intacte pour compat.

Le token de join-link est une capacité courte durée (actuellement 30 min) sans
PII : facts obligatoires `organization/workspace/session`, `actor("guest-link",
"guest_link")`, `role("guest_link")`, `capability("participant_join")`, puis
`check if time < exp`. L’authorizer exige également l’opération exacte
`participant_join`, puis termine par `deny if true`. Le connect WS legacy
continue d’accepter ses tokens historiques via `?token=` dans la query.

`POST /join/{session_id}/participants` exige `Authorization: Bearer <token>`
avant le parsing du body. Le body JSON ne contient qu’un `name` borné :
`trim()` 1..24 caractères, octets bornés, contrôles refusés. La même validation
s’applique au nom encore transporté par la query du WS legacy. La route répond
en `no-store`; un bucket dédié borne les tentatives avant l’exécution Biscuit et
une limite de concurrence extérieure borne auth, body et mutation ensemble.

Les snapshots personnalisés appliquent leurs invariants à la construction,
désérialisation et sérialisation. Le roster et le leaderboard sont plafonnés à
32 entrées, la heatmap à 64 ; une phase `asking` ne peut sérialiser aucun reveal.

## Migration

- `join_url` legacy reste publié pour l’instant ; `secure_join_url` est un champ
  additif.
- Les clients futurs doivent lire le token en mémoire depuis le fragment et ne
  pas le persister.
- `/ws/{session_id}` garde la query `?token=` pour compat ; ne pas prétendre
  avoir migré le WS.
- La redemption legacy `/sessions/{session_id}/participants` reste ouverte et
  transitoire ; la migration canonique n’est pas terminée.
- Aucune claim de complétude sur #35 n’est faite ici.
