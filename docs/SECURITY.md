# État des contrôles de sécurité

Contrôles locaux du 15 novembre 2026, sur une image ARM64. Ils décrivent les résultats observés à cette date ; refaire les scans avant une publication.

- Audit npm : zéro vulnérabilité connue dans les dépendances de production après installation du lockfile.
- Distroless Debian 13 : aucun résultat HIGH/CRITICAL corrigeable signalé par Trivy sur l’image construite.
- CLI Docker 29.9.3 : aucun résultat HIGH/CRITICAL corrigeable signalé.
- Compose 5.7.1 et Buildx 0.39.0 : deux résultats HIGH restent présents dans les binaires officiels copiés depuis `docker:29.9.3-cli`.

| Binaire | Dépendance | Résultat Trivy | Version embarquée | Version de dépendance corrigée |
|---|---|---|---|---|
| Buildx | `github.com/moby/go-archive` | CVE-2026-17106 : extraction d’archives et suivi de liens | 0.2.0 | 0.3.0 |
| Compose | `golang.org/x/net` | CVE-2026-78661 : déni de service dans l’analyse HTML | 0.60.0 | 0.62.0 |

Les binaires ont été mis à jour depuis le premier bundle Docker 29.7.0, qui produisait davantage de résultats HIGH. Les versions retenues correspondent aux versions stables observées sur les pages officielles [Compose](https://github.com/docker/compose/releases/tag/v5.7.1) et [Buildx](https://github.com/docker/buildx/releases/tag/v0.39.0).

Ces deux résultats ne sont **pas ignorés** et ne sont pas déclarés inapplicables. Le workflow Trivy est volontairement bloquant sur HIGH/CRITICAL corrigeable ; il signalera donc ces dépendances tant que le bundle officiel ne les aura pas mises à jour. Une reconstruction personnalisée des plugins avec des dépendances modifiées nécessiterait une validation distincte de ces logiciels tiers. Ne pas présenter cette image comme exempte de vulnérabilités.

La surface d’administration reste locale : publication sur 127.0.0.1, Host/Origin contrôlés, CSRF obligatoire, pas de CORS, processus non-root, rootfs en lecture seule, capacités supprimées et `no-new-privileges`. Ces mesures ne réduisent pas les pouvoirs de l’API Docker accessible via le socket. Les projets et les utilisateurs autorisés à piloter ce moteur doivent rester de confiance.

Les tests d’image vérifient l’exécution sous UID 65532 et sous l’UID non-root de l’hôte, l’accès au socket par GID, l’absence de shell, le healthcheck, les outils embarqués et l’édition YAML. Le montage des registres privés reste explicite et en lecture seule ; leurs identifiants ne sont pas copiés dans SQLite.
