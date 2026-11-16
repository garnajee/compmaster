# Validation et limites

Cette page distingue les contrôles automatisés des comportements nécessitant une vérification sur chaque plateforme. Ne pas interpréter une compilation multi-architecture comme une validation de tous les montages hôte.

Validation locale du 15 novembre 2026 : 14 tests Rust, le test Docker explicitement activé, 8 tests navigateur desktop/mobile et 11 scénarios API ont passé. Le script d’installation et l’image distroless ont été testés avec Colima ARM64, y compris build/up/down depuis le conteneur, rootfs en lecture seule, UID 65532 et UID de l’hôte. La construction des architectures linux/amd64 et linux/arm64 et leur export OCI ont également abouti. Les autres environnements de la matrice restent à exercer sur leurs machines cibles. Le scan de sécurité conserve les deux résultats HIGH décrits dans [SECURITY.md](SECURITY.md).

## Contrôles automatisés

- Rust : sécurité Host/Origin/CSRF, découverte récursive et exclusions, liens symboliques, validation impossible, regroupement Compose, persistance SQLite et opérations interrompues, ports IPv4/IPv6 et conflits internes.
- Navigateur : recherche, filtres, disposition desktop/mobile, confirmations et CSRF, navigation depuis un conflit de port vers l’éditeur, diff et conservation des commentaires.
- Docker réel : cycle d’un conteneur temporaire, réseau et volume avec nettoyage exact des ressources créées.
- API sur Docker réel : fichiers de noms personnalisés, rapprochement après lancement, blocage des ports, conflit de révision, validation YAML invalide, sauvegarde atomique, logs SSE, terminal WebSocket, remplacement puis rollback, ressources et exclusion des backups du nettoyage.

```sh
cargo test
npm --prefix frontend test
npm --prefix frontend run build
# Installer Chromium une fois si absent : cd frontend && npx playwright install chromium
npm --prefix frontend run test:e2e

# Utiliser exclusivement un moteur de développement.
export DOCKER_HOST="$(docker context inspect --format '{{.Endpoints.docker.Host}}')"
export COMPMASTER_DOCKER_TESTS=1
cargo test -- --ignored
cargo build
node scripts/test-api.mjs
```

Si le plugin Compose est installé dans un emplacement non standard, définir `DOCKER_CLI` vers un wrapper qui appelle Docker avec une configuration contenant `cliPluginsExtraDirs`. Le script API conserve ses fichiers temporaires pour inspection et supprime ses seules ressources `cm-test-*`, même en cas d’échec. Ne pas lancer deux suites modifiant le même moteur en parallèle avec une intervention manuelle sur leurs ressources.

## Image

L’image doit être construite pour amd64 et arm64, puis vérifiée avec un moteur Linux : UID/GID 65532, commandes Docker/Compose/Buildx exécutables sans shell, système de fichiers racine en lecture seule, volume `/data` inscriptible, socket accessible via son groupe, healthcheck positif.

Les versions de distroless et du CLI sont épinglées par digest d’index multi-architecture. Les mettre à jour délibérément après vérification du registre. Le workflow CI construit une image native et une image multi-architecture ; il effectue également un scan de vulnérabilités. Le scan du code npm peut être lancé avec `npm --prefix frontend audit`.

## Points à vérifier sur les machines cibles

| Plateforme | Contrôle essentiel |
|---|---|
| Linux Engine | GID du socket, permissions de lecture/écriture des projets, chemins identiques et accès aux builds/bind mounts |
| Docker rootless | Socket sous `/run/user/<uid>/docker.sock`, accès du daemon aux fichiers et ressources du même utilisateur |
| Docker Desktop macOS | Partage du dossier hôte, socket `.raw`, permissions vues depuis la VM et bind mounts relatifs |
| Colima macOS | Socket du daemon dans la VM, partage du dossier `/Users`, résolution de l’hôte pour les sondages |
| Windows WSL2 | Docker Engine dans la distribution pour une parité Linux ; vérifier tout partage de chemins si Docker Desktop est utilisé |

Le montage RW n’accorde aucun droit supplémentaire sur les fichiers. Le script configure `COMPMASTER_UID`/`COMPMASTER_GID` avec votre compte hôte non-root ; l’image conserve 65532 comme défaut pour une utilisation directe. Si vous choisissez un autre UID, accordez-lui un accès adapté et tenez compte du propriétaire après remplacement atomique des fichiers. CompMaster ne fait pas de `chmod` ou de `chown` récursif sur vos projets. Le script de configuration peut lancer un conteneur Alpine éphémère pour lire le GID du socket de la VM macOS ; il ne modifie pas ce socket.

## Limites explicites

- Un moteur local Unix uniquement. Windows natif, Swarm, Kubernetes et les moteurs distants sont hors v1.
- La parité WSL2 exige que le daemon voie les projets au même chemin Linux que CompMaster. Une simple intégration Docker Desktop/WSL2 ne garantit pas cette propriété : les chemins réécrits dans une VM doivent être vérifiés ; utiliser Docker Engine dans la distribution pour le cas Linux cohérent.
- Un YAML valide syntaxiquement n’est pas nécessairement un Compose exécutable ; la validation officielle reste obligatoire.
- Les options originales du shell, profils et variables d’environnement exportées ne sont pas récupérables de façon générale depuis Docker. Les actions utilisent les fichiers et l’environnement actuellement disponibles. Les chemins et services ambigus désactivent les actions Compose.
- Le sondage TCP de l’hôte est indicatif, notamment pour localhost, IPv6, UDP et les VM. Docker peut encore refuser un port après un précontrôle réussi. Les ports dynamiques sont relus depuis `NetworkSettings.Ports` après lancement.
- La réconciliation sonde au maximum 100 ports TCP distincts demandés, avec concurrence bornée. Le bouton de sondage permet de vérifier d’autres ports. Les lignes « service hôte » ne prouvent pas une adresse d’écoute précise.
- Le rollback restaure le conteneur et son nom, pas les écritures effectuées par l’application dans les volumes partagés. Une erreur de restauration est exposée dans le travail. Les backups nécessitent une suppression explicite.
- Pas d’identification des processus hôte, de récupération heuristique des Dockerfile ou d’exploration des données des volumes.
- Les images utilisant des helpers d’identifiants natifs incompatibles produisent un diagnostic. Aucun secret de registre n’est conservé dans SQLite.
- Les logs et événements du moteur sont bornés en mémoire ; l’historique des travaux est persistant. Les métriques utilisent des snapshots périodiques et ne constituent pas une série historique.
