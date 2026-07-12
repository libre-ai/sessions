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
- espace : 32 documents et 4 Mio de charge mémoire conservatrice;
- processus : 256 documents et 32 Mio de charge mémoire conservatrice;
- MIME fermé : `text/plain` avec `.txt`, `text/markdown` avec `.md`/`.markdown`;
- nom sans chemin, contrôle/NUL, composant caché ou `..`; contenu JSON UTF-8 non vide;
- body JSON : 2 Mio maximum; erreurs externes stables `400`, `413`, `507`, `503`;
- aucune éviction : l’insertion, le dédoublonnage et les capacités sont atomiques;
- hashing, validation et découpage sont exécutés hors verrou; aucun `await` sous verrou.

La charge réserve conservativement deux fois la capacité des chaînes de l’upload,
les ranges de chunks et un overhead fixe, y compris lorsque le corps Pending est
ensuite jeté. Le store ne conserve donc que le texte Approved nécessaire au
retrieval; les réponses GET/upload ne contiennent jamais le corps.

## Authentification et exploitation

`GET /api/corpus/documents` exige `read`. `POST` exige `add_document`, le contrôle
CSRF global et une relecture de la membership actuelle : un token valide mais
révoqué est refusé. L’espace est exclusivement celui de la session owner.

Le store partage les mêmes limites que les sessions owner : mémoire d’un seul
processus, perte complète au redémarrage, aucun support multi-instance. Il ne faut
pas masquer cette instance derrière un load balancer. Aucun contenu, prompt,
token ou PII brut n’est loggé.
