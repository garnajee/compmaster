# CompMaster

CompMaster découvre récursivement vos fichiers Compose et les rapproche des conteneurs de votre moteur Docker local. Un YAML peut porter n’importe quel nom ; plusieurs projets peuvent vivre dans un même dossier.

Backend Rust/Axum, interface React/TypeScript, SQLite, image distroless non-root. Linux, macOS avec Docker Desktop ou Colima et Windows via WSL2, pour des **conteneurs Linux**.

## Démarrer

Prérequis : moteur Docker démarré, CLI Docker et plugin Compose.

```sh
git clone https://github.com/garnajee/compmaster.git
cd compmaster
bash scripts/setup.sh /chemin/absolu/vers/mes-projets
bash scripts/compose.sh up -d --build
```

Ouvrir **http://127.0.0.1:8080** (ou le port libre indiqué par le script si 8080 est occupé). Le script crée `.env` sans écraser une configuration existante. Sous WSL2, lancer ces commandes dans la distribution Linux avec Docker Engine et conserver les projets dans son système de fichiers Linux. Avec Docker Desktop/WSL2, vérifier que le daemon et CompMaster voient les bind mounts au même chemin ; l’intégration seule ne garantit pas cette propriété. Les conteneurs Windows natifs et les moteurs distants ne sont pas pris en charge.

Le volume nommé `compmaster-data` reçoit la base et les sauvegardes. L’image utilise l’UID 65532 par défaut ; le script configure l’UID/GID non-root de votre compte hôte pour que l’édition conserve le propriétaire habituel des YAML. Le groupe supplémentaire 65532 permet l’accès au volume de données. Sauvegardez ce volume avant toute mise à jour. `docker compose down` le conserve ; ne pas ajouter `-v` si vous souhaitez garder vos données.

Le dossier des projets est monté en lecture-écriture **au même chemin absolu** dans CompMaster. Les chemins relatifs des builds, `.env`, `env_file` et bind mounts conservent ainsi leur sens. Tout chemin extérieur à cette racine utilisé par un projet doit être monté explicitement au même emplacement. Aucun fichier externe n’est monté automatiquement.

L’app écoute uniquement sur l’interface locale grâce à la publication `127.0.0.1`. Elle n’a pas d’authentification : ne changez pas cette publication pour l’exposer sur le LAN ou Internet. Un accès au socket permet d’administrer le moteur et indirectement l’hôte, même avec un processus non-root.

## Fonctionnement

- Scan de tous les `.yml` et `.yaml`, sans suivre les symlinks. Les YAML avec une section `services` sont validés par `docker compose config`; les autres YAML valides sont ignorés. Les fichiers malformés restent visibles avec leurs diagnostics.
- Chaque fichier Compose non lancé constitue un projet. Sans `name`, le nom `cm-<dossier>-<fichier>-<hash8>` évite les collisions.
- Les labels `com.docker.compose.project` et `.service` regroupent les conteneurs existants sans les modifier. Les chemins enregistrés dans les labels sont des indices : les fichiers doivent être accessibles, leur configuration combinée valide et leurs services cohérents avant d’autoriser les commandes Compose. En cas d’ambiguïté, le projet reste en mode runtime uniquement.
- Les déploiements multi-`-f` vérifiés conservent l’ordre de leurs fichiers. L’éditeur permet de sélectionner chaque fichier et valide l’ensemble avant enregistrement.
- `.compmasterignore` suit la syntaxe `.gitignore`. Les exclusions sont prises en compte à chaque scan. Les modifications YAML et `.env`, les événements Docker et une réconciliation toutes les 30 secondes actualisent l’inventaire.
- Pull, build, up, start, stop, restart et down sont des travaux persistants. Les opérations de mutation du moteur sont sérialisées. `down` ne supprime pas implicitement les volumes.
- Les logs suivent stdout/stderr avec un maximum de 5 000 lignes affichées. Le terminal ouvre uniquement un processus dans le conteneur sélectionné ; les images distroless des projets peuvent ne pas contenir de shell.
- Les enregistrements YAML vérifient la révision pour éviter d’écraser un changement externe, valident la syntaxe et Compose, sauvegardent l’original puis remplacent le fichier atomiquement.
- Les suppressions sont confirmées. Le nettoyage ciblé propose uniquement des conteneurs arrêtés, images dangling et réseaux/volumes inutilisés ; le daemon vérifie à nouveau leurs références au moment de la suppression.

## Ports

La page Ports rapproche les ports réellement publiés, les réservations de conteneurs arrêtés et les déclarations Compose. Les protocoles TCP/UDP et IP d’écoute restent distincts ; les wildcard IPv4/IPv6 sont traitées conservativement.

Avant démarrage, les conflits certains avec un autre conteneur ou un service TCP détecté bloquent l’opération. Les projets arrêtés et les YAML non lancés donnent des conflits potentiels : ils ne possèdent pas encore de socket actif.

Le sondage ouvre une connexion TCP sans envoyer de données. Il passe par `host.docker.internal`, ou `COMPMASTER_PROBE_HOST` en développement. Un refus explicite signifie « probablement libre » ; une expiration ou une erreur DNS signifie « inconnu ». Il ne permet pas d’identifier le processus hôte ni de garantir la disponibilité sur toutes les interfaces, notamment localhost, IPv6 et UDP. Docker reste l’arbitre final et peut détecter une course au moment de publier le port.

Les propositions de ports ne sont jamais appliquées silencieusement. L’éditeur ne prépare un remplacement automatique que pour une déclaration courte entre guillemets, unique et sans ambiguïté ; les autres syntaxes doivent être modifiées manuellement puis validées.

## Remplacer un conteneur autonome

Le formulaire propose un diff de la configuration Docker : image, commande, environnement, ports, montages, restart policy et limites CPU/mémoire. CompMaster arrête et renomme l’ancien conteneur, crée le nouveau, puis vérifie son état et son éventuel healthcheck. Un échec déclenche une tentative de retour à l’ancien conteneur. L’original est conservé sous un nom `*-cm-backup-*` après succès et doit être supprimé séparément.

Les conteneurs `AutoRemove` sont refusés, car ils ne permettraient pas de retour arrière. Les volumes existants sont réutilisés ; cette opération n’est pas une sauvegarde des données ni une garantie de rollback d’une migration applicative. Les changements réseau utilisent les réseaux existants de l’ancien conteneur ; la connexion et déconnexion de réseaux disposent d’actions dédiées.

## Registres privés

Créer un dossier de configuration dédié contenant un `config.json` avec des entrées `auths`, accessible par l’UID 65532 ou un groupe supplémentaire configuré. Ajouter `DOCKER_CONFIG_DIR='/chemin/absolu/vers/config'` dans `.env`, puis :

```sh
bash scripts/compose.sh -f compose.yaml -f compose.registry.yaml up -d --build
```

Le montage est en lecture seule. Les helpers `desktop`, `osxkeychain` et `wincred` ne fonctionnent pas dans un conteneur Linux et sont signalés. Un helper Linux compatible peut être monté explicitement dans le PATH. Aucun identifiant n’est importé dans SQLite.

Les vérifications d’images utilisent le registre, avec un cache de six heures. Une image sans digest, un registre inaccessible ou une identité native indisponible produit un statut local/inconnu ou un diagnostic. Aucun pull ou redéploiement automatique n’est déclenché.

## Développement

Rust stable récent, Node.js 24+, Docker et Compose pour les tests d’intégration.

```sh
npm --prefix frontend ci
npm --prefix frontend run build
COMPOSE_ROOT=/chemin/projets DOCKER_HOST=unix:///chemin/docker.sock cargo run
```

Par défaut le serveur sert `frontend/dist` sur `127.0.0.1:8080`. Pour Vite :

```sh
# Terminal 1
COMPMASTER_PUBLIC_ORIGIN=http://127.0.0.1:5173 cargo run
# Terminal 2
npm --prefix frontend run dev -- --host 127.0.0.1
```

En développement, les contextes Docker de l’hôte ne sont pas devinés : définir `DOCKER_HOST` avec `docker context inspect --format '{{.Endpoints.docker.Host}}'` si nécessaire. Si le plugin Compose n’est pas découvert par le CLI, configurer `cliPluginsExtraDirs` dans votre configuration Docker ou utiliser `DOCKER_CLI` pour un wrapper dédié.

Variables : `COMPOSE_ROOT`, `DOCKER_HOST`, `DOCKER_CLI`, `DOCKER_CONFIG`, `COMPMASTER_LISTEN`, `COMPMASTER_PUBLIC_ORIGIN`, `COMPMASTER_DATA_DIR`, `COMPMASTER_STATIC_DIR`, `COMPMASTER_PROBE_HOST`, `COMPMASTER_SCAN_CONCURRENCY`, `RUST_LOG`.

## Vérifications

```sh
cargo test
npm --prefix frontend test
npm --prefix frontend run build
npm --prefix frontend run test:e2e
docker build -t compmaster:test .
docker buildx build --platform linux/amd64,linux/arm64 .
```

Les tests Docker ignorés par défaut doivent être activés explicitement avec `COMPMASTER_DOCKER_TESTS=1 cargo test -- --ignored`. Ils créent uniquement des ressources préfixées `cm-test-` et nettoient leurs propres ressources. Ne jamais utiliser un moteur de production pour des tests de remplacement ou de suppression.

L’API versionnée est documentée dans [docs/API.md](docs/API.md). Les limites connues et les vérifications par plateforme sont suivies dans [docs/VALIDATION.md](docs/VALIDATION.md).

Le scan de l’image signale encore deux vulnérabilités HIGH dans les plugins officiels Compose/Buildx, après mise à jour vers le bundle stable observé. Elles sont documentées dans [docs/SECURITY.md](docs/SECURITY.md) et restent bloquantes en CI ; aucune exclusion silencieuse n’a été ajoutée.

## Licence

GNU GPL v3 — voir [LICENSE](LICENSE). Fourni sans garantie.
