# Corpus owner process-local

- **Statut :** verticale déterministe de l’issue #34
- **Portée :** liste/upload UTF-8 `text/plain` et `text/markdown`, retrieval owner exact
- **Non-portée :** persistance, multi-instance, suppression, PDF/DOCX, OCR, URL distante

## Autorité et propriété de sécurité

Un upload arbitraire est toujours `Pending`. Il n’est jamais exposé au retriever
Notebook et ne peut donc jamais produire `Grounded`, même si sa source demande au
provider de répondre `supported=true`. Le provider, le nom utilisateur et le
contenu source ne créent ni ne sélectionnent un permit.

Une unique fixture Markdown est pré-approuvée dans `approved_claims.rs` par ses
octets exacts et son SHA-256. Le fichier exact devient `Approved`; BOM, CRLF,
newline ajouté/retiré ou tout autre octet donne `Pending`. Lors d’une requête sur
le claim associé, le registre vérifie que ce document exact existe encore dans
l’espace authentifié puis émet un permit opaque lié à l’espace, au document, au
chunk, à la révision et aux hashes. Le pipeline réel exécute ensuite retrieval,
génération et vérification; seul le gate final #33 peut projeter `Grounded`.

Cette propriété de hash prouve l’appartenance à un artefact pré-approuvé, **pas sa
vérité**, son actualité ou un entailment sémantique général. Le titre de citation
est canonique et ne vient jamais du nom de fichier fourni par l’utilisateur.

## Bornes et validation

- fichier : 256 Kio; nom UTF-8 : 128 octets; 128 chunks maximum;
- espace : 32 documents et 4 Mio de mémoire réellement retenue;
- processus : 256 documents et 32 Mio de mémoire réellement retenue;
- MIME fermé : `text/plain` avec `.txt`, `text/markdown` avec `.md`/`.markdown`;
- nom sans chemin, contrôle/NUL, composant caché ou `..`; contenu JSON UTF-8 non vide;
- body JSON : 2 Mio maximum; erreurs externes stables `400`, `413`, `507`, `503`;
- 4 uploads maximum en concurrence sur le processus, permit conservé pendant
  extraction JSON et préparation; authz exécutée avant tout polling du body;
- aucune éviction : l’insertion, le dédoublonnage et les capacités sont atomiques;
- hashing, validation et découpage sont exécutés hors verrou; aucun `await` sous verrou.

La mémoire durable compte uniquement les allocations réellement retenues : nom,
MIME, hash, identifiant/overhead, puis corps et ranges seulement pour `Approved`.
Pour `Pending`, le corps et les chunks préparés sont jetés, `chunk_count` vaut
strictement `0`, et leur mémoire transitoire relève des limites body+concurrence,
non d’un quota historique fictif. Les réponses GET/upload ne contiennent jamais
le corps.

## Authentification et exploitation

`GET /api/corpus/documents` et `POST /api/rag/query` exigent `read`; `POST` upload
exige `add_document`. Chacun relit la membership actuelle après validation du
cookie et du Biscuit. Le middleware upload place l’owner authentifié dans les
extensions avant l’extracteur JSON. Le CSRF global reste plus externe et s’exécute
donc avant cette authz sur toute requête cookie unsafe.

La révocation est fail-closed au début de chaque requête. Upload refait aussi le
contrôle après préparation, juste avant insert; query le refait après le pipeline/
timeout, juste avant toute projection potentielle `Grounded`. Une requête en vol
n’est pas annulée atomiquement au moment exact d’une révocation : du travail peut
terminer, mais ces seconds contrôles empêchent l’insert/publication si la
révocation est déjà visible à leur instant. Une révocation concurrente après le
dernier contrôle garde la sémantique classique « autorisée au dernier check ».
L’espace est exclusivement celui de la session owner.

Le store partage les mêmes limites que les sessions owner : mémoire d’un seul
processus, perte complète au redémarrage, aucun support multi-instance. Il ne faut
pas masquer cette instance derrière un load balancer. Le succès d’upload produit
uniquement un audit structuré avec identifiant acteur pseudonyme, espace opaque,
identifiant document généré serveur, statut et déduplication. Aucun filename,
contenu, sujet OIDC, prompt, token, PII brute ou erreur backend brute n’est loggé.
