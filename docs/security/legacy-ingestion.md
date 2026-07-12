# Migration de l’ingestion live legacy

`POST /corpus/documents` est une frontière distincte de l’API owner. Elle ingère
vers l’espace live `default` et **n’est plus ouverte en développement**.

## Configuration obligatoire

Le processus refuse de démarrer sans `INGEST_TOKEN`. La valeur doit contenir de
32 à 512 octets ASCII graphiques compatibles avec un header HTTP (`0x21..0x7e`);
Unicode, espace, octets obs-text et DEL sont refusés. Générer par
exemple `openssl rand -hex 32`, puis l’injecter via le gestionnaire de secrets du
déploiement. Ne jamais la committer ni la logger.

Chaque appel doit envoyer `Authorization: Bearer <INGEST_TOKEN>`. Absence, token
faible configuré ou mauvais token échoue fermé en `401`. Le middleware compare
des digests SHA-256 de longueur fixe en temps constant **avant** de poller le
body. Le handler ne connaît plus le secret.

## Bornes et ordre des layers

Le guard global CSRF reste le plus externe pour les requêtes unsafe portant un
cookie owner. Sur cette route bearer, le middleware token est ensuite externe à
une limite globale de 4 ingestions concurrentes, elle-même externe à
l’extracteur `Bytes` limité à 1 Mio. Ainsi un appel non authentifié ne peut ni
bufferiser le body, ni déclencher parsing, embeddings ou stockage.

Les erreurs backend et le `document_id` fourni par l’appelant ne sont pas
journalisés : ils peuvent contenir des détails internes ou de la PII. La réponse
reste bornée et opaque.

## Procédure de migration

1. Générer et distribuer `INGEST_TOKEN` avant de déployer cette version.
2. Mettre à jour tous les producteurs pour envoyer le Bearer.
3. Vérifier un appel autorisé, puis les preuves `401` sans/mauvais token et `413`
   au-delà de 1 Mio.
4. Faire tourner le token comme tout secret statique et révoquer l’ancien côté
   producteurs et serveur sans période d’ouverture anonyme.
