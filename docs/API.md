# API CompMaster v1

L’API est locale et partage l’origine du frontend. Toutes les réponses sont JSON, sauf les flux SSE et WebSocket. Les erreurs ont la forme `{"error":"diagnostic"}`. Un conflit de révision renvoie HTTP 409, une erreur de sécurité HTTP 403, les erreurs de validation HTTP 400.

Chaque requête doit utiliser le `Host` configuré dans `COMPMASTER_PUBLIC_ORIGIN`. Les écritures demandent `X-CSRF-Token`, obtenu via `GET /api/v1/session`. Si `Origin` est présent, il doit correspondre exactement à l’origine configurée. Le terminal exige également cette origine et le jeton dans son URL. Aucun CORS n’est activé.

| Méthode | Route | Usage |
|---|---|---|
| GET | `/api/v1/session` | Version, racine, jeton CSRF, diagnostics des helpers de credentials |
| GET | `/api/v1/inventory` | Projets, conteneurs autonomes, images, réseaux, volumes, ports, compteurs et derniers événements Docker |
| POST | `/api/v1/refresh` | Refaire le scan et rapprocher l’état du moteur |
| GET | `/api/v1/events` | SSE : événements `inventory`, `job` et `refresh` |
| GET | `/api/v1/jobs` | Historique des 500 derniers travaux |
| POST | `/api/v1/jobs` | Créer un travail asynchrone |
| GET, PUT | `/api/v1/settings` | Favoris `favorite:<projectId>`, thème et filtres |
| GET, PUT | `/api/v1/projects/{id}/file?index=0` | Lire ou enregistrer un fichier d’un projet ; index dans l’ordre `-f` |
| POST | `/api/v1/projects/{id}/validate?index=0` | Valider une proposition YAML dans l’ensemble multi-fichiers |
| GET | `/api/v1/inspect/{kind}/{id}` | Inspection Docker native ; kind = container, image, network ou volume |
| GET | `/api/v1/containers/{id}/stats` | Snapshot CPU/mémoire au format JSON du CLI Docker |
| GET | `/api/v1/containers/{id}/logs` | SSE `log` avec `{stream,line}`, puis `end` |
| POST | `/api/v1/ports/probe` | Sondage TCP de 100 ports maximum |
| GET | `/api/v1/prune/preview` | Liste des ressources éligibles au nettoyage ciblé |
| POST | `/api/v1/prune` | Créer un travail de suppression de ressources sélectionnées |
| WS | `/ws/terminal/{id}?token=…&command=/bin/sh` | Terminal interactif dans un conteneur existant |

Les identifiants de projet peuvent contenir `:` ; toujours encoder les segments d’URL.

## Travaux

```json
{
  "target": "cmp_0123456789abcdef",
  "action": "up",
  "options": {}
}
```

Réponse : `Job { id, target, action, state, progress, output, error, createdAt, finishedAt }`. `state` vaut `queued`, `running`, `succeeded`, `failed` ou `cancelled`. Les sorties des commandes Compose et des opérations conteneur sont transmises au fil de l’exécution ; les 500 dernières lignes sont conservées. Une commande est limitée à 15 minutes. L’état d’une opération interrompue par un redémarrage devient `cancelled`, avec un diagnostic demandant de vérifier le moteur.

Le champ `progress` est une phase indicative (0 = attente, 10 = validation, 50 = exécution, 100 = fin), et non un pourcentage précis du téléchargement ou du build.

| Cible | Actions |
|---|---|
| ID de projet vérifié | `pull`, `build`, `up`, `up_build`, `start`, `stop`, `restart`, `down` |
| ID complet de conteneur | `start`, `stop`, `restart`, `pause`, `unpause`, `kill`, `rename`, `replace`, `remove` |
| `image:<id>` | `check_update`, `remove` |
| `network:<id>` | `connect`, `disconnect`, `remove` |
| `volume:<name>` | `remove` |
| `network:new` | `create_network` |
| `volume:new` | `create_volume` |

Les actions destructives, stop/restart/pause/kill, rename, replace et déconnexion demandent `options.confirmed: true`. `rename` ajoute `name`, les connexions réseau ajoutent `container`. La création d’une ressource demande `name` et accepte `driver` et `driverOptions` (objet chaîne → chaîne). Pour les réseaux : `subnet`, `gateway`, `internal`, `ipv6`, `attachable` sont facultatifs. `check_update` accepte `force:true` pour contourner son cache de six heures.

Pour `replace`, `config` accepte seulement `Image`, `Cmd`, `Entrypoint`, `Env`, `Healthcheck`, `WorkingDir`, `User`. `hostConfig` accepte `PortBindings`, `Binds`, `Mounts`, `RestartPolicy`, `Memory`, `NanoCpus`. Les formes sont celles de l’API Docker. Les autres propriétés sont conservées depuis l’inspection du conteneur. Les volumes existants, y compris anonymes, sont réutilisés. Les conteneurs Compose et AutoRemove ne sont pas remplaçables par cette route. La sauvegarde renommée est enregistrée dans SQLite et exclue du nettoyage ciblé.

## Édition

GET renvoie `{content,revision,path,files}`. PUT et validation acceptent :

```json
{"content":"services:\n  app:\n    image: alpine\n","revision":"sha256-du-contenu-lu"}
```

PUT exige la bonne révision, contrôle la syntaxe et exécute `docker compose config`, sauvegarde l’original et remplace le fichier atomiquement. Les fichiers restent confinés à `COMPOSE_ROOT`. Les permissions de l’ancien fichier sont conservées. Le montage RW ne remplace pas les permissions du système de fichiers ; un accès refusé produit un diagnostic.

## Ports et nettoyage

Le sondage accepte `{"ports":[8000,8080]}` et renvoie `{host,bestEffort,results:[{port,status,suggestedPort}]}`. Les ports connus de Docker sont distingués des services externes. Les suggestions sondées ne constituent pas une réservation.

Les bindings d’inventaire portent `hostIp`, `hostPort`, `hostPortEnd` (fin facultative d’une plage dynamique), `containerPort`, `protocol`, `ownerId`, `ownerName`, `service`, `source`, `status` et `conflict`. Les plages courte syntaxe sont développées par Compose ; les plages de publication dynamiques restent une plage. Le précontrôle n’interdit une plage que si tous ses ports sont certainement occupés. Un budget de trois secondes limite le sondage des grandes plages.

Le nettoyage accepte `{"confirmed":true,"resources":[{"kind":"volume","id":"nom"}]}` et renvoie un Job. L’éligibilité est revérifiée pour chaque ressource, les suppressions ne forcent jamais le daemon.

## Terminal

Le serveur envoie les octets de sortie sous forme de frames binaires. Le client envoie les entrées en frames binaires UTF-8 et les redimensionnements en frames texte JSON : `{"cols":120,"rows":35}`. La commande est un chemin d’exécutable, sans interprétation shell. Pour des commandes composées, ouvrir un shell présent dans le conteneur puis les taper. La fermeture de la connexion termine l’attachement au processus ; Docker peut conserver le processus exec jusqu’à sa sortie. Utiliser `exit` pour fermer proprement le shell.
