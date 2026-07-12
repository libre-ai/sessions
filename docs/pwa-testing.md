# Vérifier le shell PWA owner

Cette procédure produit une **preuve locale ou sur une cible de test fournie**. Elle ne prouve pas qu’un déploiement public existe. Aucun URL Clever Cloud n’est présumé disponible.

## Limite hors ligne et sécurité

Le service worker sous scope `/app/` précache uniquement le document shell, les assets WASM/JS/CSS versionnés, le manifest et les icônes locales. Il ne met jamais en cache auth, API, corpus, RAG, sessions, WebSocket, requêtes non-GET, requêtes avec `Authorization` ou origines tierces. Hors ligne, `/app/notebook` affiche donc le shell puis un état backend indisponible; aucune réponse, citation, identité ou donnée owner ancienne n’est fournie.

Une mise à jour du worker ne force pas `skipWaiting`: elle s’active simplement après fermeture de tous les onglets/clients `/app`. Ce comportement évite de remplacer le runtime pendant une interaction.

## Preuve automatisée locale

```bash
./scripts/build-owner-app.sh
cd e2e
npm ci
npx playwright install chromium firefox webkit
npm test
```

Chromium exécute la suite fonctionnelle complète et les assertions installabilité/Cache Storage. Firefox, WebKit et un profil mobile Chromium exécutent seulement `pwa-smoke.spec.ts` afin de vérifier WASM, navigation profonde, CSP, console et worker sans multiplier la suite.

Pour une cible HTTPS de test déjà fournie, Playwright ne démarre pas de serveur local:

```bash
cd e2e
TEST_BASE_URL=https://cible-de-test.example npx playwright test tests/pwa-smoke.spec.ts \
  --project=chromium --project=firefox-smoke --project=webkit-smoke \
  --project=mobile-chromium-smoke
```

Cette commande distante reste volontairement ciblée sur `pwa-smoke.spec.ts`; elle ne lance pas la suite Chromium complète, dont certains scénarios sont mutateurs. Le contrôle local complet reste `npm test`. N’utiliser qu’une cible isolée autorisée. Les mocks Playwright prouvent le comportement client, pas la disponibilité réelle de ses API. La garde Keycloak réelle reste la procédure manuelle distincte de [`e2e-testing.md`](e2e-testing.md).

## Android Chrome

1. Servir le checkout sur une origine HTTPS accessible au téléphone, ou utiliser Chrome sur `localhost` via redirection de port USB; une adresse LAN en HTTP n’est pas un contexte sécurisé suffisant.
2. Ouvrir `https://hôte/app/`, vérifier le nom et les icônes dans **Installer l’application** / **Ajouter à l’écran d’accueil**.
3. Lancer l’icône et vérifier l’affichage `standalone` et la navigation `/app/notebook`.
4. Charger une fois le shell, passer l’appareil hors ligne, rouvrir `/app/notebook`: le shell doit s’afficher et les données/API doivent rester indisponibles.
5. Revenir en ligne et fermer tous les onglets de l’app pour activer une éventuelle mise à jour du worker.

## iOS Safari

1. Utiliser une origine HTTPS accessible à l’iPhone/iPad; Safari ne traite pas une adresse LAN HTTP comme `localhost`.
2. Ouvrir `/app/` dans Safari, puis **Partager → Sur l’écran d’accueil**. iOS utilise l’icône Apple locale 180×180.
3. Ouvrir depuis l’écran d’accueil et vérifier le mode autonome et une route profonde.
4. Après un premier chargement en ligne, couper le réseau: seul le shell doit rester visible. Toute requête API/RAG/corpus doit échouer sans résultat obsolète.
5. Pour une nouvelle version, remettre le réseau puis fermer toutes les fenêtres de l’app avant de la rouvrir.

## Notes Clever Cloud, sans déploiement

Une éventuelle cible de test Clever doit servir HTTPS, conserver `Service-Worker-Allowed: /app/` sur `/app/sw.js`, et ne pas réécrire les MIME ou headers de cache/sécurité. Exécuter alors le smoke avec `TEST_BASE_URL`. Ce document ne déclenche aucun déploiement, ne publie aucune URL et ne transforme pas une preuve locale en preuve de disponibilité.
