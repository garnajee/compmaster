# État des contrôles de sécurité

Historique initial : contrôles locaux du 15 novembre 2026 sur ARM64. Correctif du 17 novembre 2026 : reconstruction des plugins avec les dépendances corrigées, puis validation par Trivy dans la matrice CI amd64/arm64. Refaire les scans avant une publication.

- Audit npm : zéro vulnérabilité connue dans les dépendances de production après installation du lockfile.
- Distroless Debian 13 : aucun résultat HIGH/CRITICAL corrigeable signalé par Trivy sur l’image construite.
- CLI Docker 29.9.3 : aucun résultat HIGH/CRITICAL corrigeable signalé.
- Les binaires officiels Compose 5.7.1 et Buildx 0.39.0 du bundle `docker:29.9.3-cli` contenaient deux résultats HIGH. Ils sont désormais remplacés par les plugins reconstruits ci-dessous.

| Binaire | Dépendance | Résultat Trivy | Version embarquée | Version de dépendance corrigée |
|---|---|---|---|---|
| Buildx | `github.com/moby/go-archive` | CVE-2026-17106 : extraction d’archives et suivi de liens | 0.2.0 | 0.3.0 |
| Compose | `golang.org/x/net` | CVE-2026-78661 : déni de service dans l’analyse HTML | 0.60.0 | 0.62.0 |

Les binaires ont été mis à jour depuis le premier bundle Docker 29.7.0, qui produisait davantage de résultats HIGH. Les versions retenues correspondent aux versions stables observées sur les pages officielles [Compose](https://github.com/docker/compose/releases/tag/v5.7.1) et [Buildx](https://github.com/docker/buildx/releases/tag/v0.39.0).

Ces résultats ne sont **pas ignorés** et ne sont pas déclarés inapplicables. Le workflow Trivy reste bloquant sur HIGH/CRITICAL corrigeable.

## Reconstruction des plugins

Le stage Go de l’image utilise une toolchain officielle épinglée par digest. `scripts/build-docker-plugin.sh` télécharge les tags officiels via le proxy Go et vérifie leurs modules avec `sum.golang.org`, puis construit des binaires Linux statiques :

- Buildx `v0.39.0-compmaster.1` : tag `v0.39.0`, avec `github.com/moby/go-archive v0.3.3`.
- Compose `v5.7.2-compmaster.1` : tag `v5.7.2`, avec `golang.org/x/net v0.62.0`.

Les autres dépendances proviennent des graphes de modules amont, avec la résolution minimale Go. Les scripts vérifient dans les métadonnées de chaque binaire que la version corrigée est effectivement liée. La toolchain et les sources Go restent dans les stages de build ; l’image finale conserve distroless, le CLI Docker officiel et les deux plugins statiques. Les tests d’image effectuent un vrai build/up/down Compose sous UID 65532 puis sous l’UID hôte, avant le scan sur chaque architecture native.

La surface d’administration reste locale : publication sur 127.0.0.1, Host/Origin contrôlés, CSRF obligatoire, pas de CORS, processus non-root, rootfs en lecture seule, capacités supprimées et `no-new-privileges`. Ces mesures ne réduisent pas les pouvoirs de l’API Docker accessible via le socket. Les projets et les utilisateurs autorisés à piloter ce moteur doivent rester de confiance.

Les tests d’image vérifient l’exécution sous UID 65532 et sous l’UID non-root de l’hôte, l’accès au socket par GID, l’absence de shell, le healthcheck, les outils embarqués et l’édition YAML. Le montage des registres privés reste explicite et en lecture seule ; leurs identifiants ne sont pas copiés dans SQLite.
