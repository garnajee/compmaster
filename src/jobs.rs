use crate::{
    AppState,
    db::Database,
    models::{Job, JobState, StartJobRequest},
    ports,
};
use chrono::Utc;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, broadcast};

#[derive(Clone)]
pub struct JobManager {
    pub events: broadcast::Sender<Job>,
    db: Database,
    limit: usize,
    locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
}
impl JobManager {
    pub fn new(db: Database, limit: usize) -> Self {
        Self {
            events: broadcast::channel(128).0,
            db,
            limit,
            locks: Default::default(),
        }
    }
    async fn publish(&self, job: &Job) {
        if let Err(error) = self.db.save_job(job, self.limit).await {
            tracing::error!(%error, "job persistence failed");
        }
        let _ = self.events.send(job.clone());
    }
    pub async fn submit(
        &self,
        state: Arc<AppState>,
        request: StartJobRequest,
    ) -> anyhow::Result<Job> {
        anyhow::ensure!(
            matches!(
                request.action.as_str(),
                "pull"
                    | "build"
                    | "up"
                    | "up_build"
                    | "start"
                    | "stop"
                    | "restart"
                    | "down"
                    | "pause"
                    | "unpause"
                    | "kill"
                    | "rename"
                    | "remove"
                    | "create_network"
                    | "create_volume"
                    | "connect"
                    | "disconnect"
                    | "replace"
                    | "check_update"
                    | "prune"
            ),
            "Action inconnue"
        );
        let job = Job {
            id: uuid::Uuid::new_v4().to_string(),
            target: request.target.clone(),
            action: request.action.clone(),
            state: JobState::Queued,
            progress: 0,
            output: vec![],
            error: None,
            created_at: Utc::now(),
            finished_at: None,
        };
        self.publish(&job).await;
        let mut running = job.clone();
        let manager = self.clone();
        tokio::spawn(async move {
            let lock = manager
                .locks
                .lock()
                .await
                .entry(request.target.clone())
                .or_default()
                .clone();
            let _guard = lock.lock().await;
            running.state = JobState::Running;
            running.progress = 10;
            manager.publish(&running).await;
            // Serialize starts across projects: checks and actual binds must not race.
            let engine_lock = manager
                .locks
                .lock()
                .await
                .entry("__engine_mutation__".into())
                .or_default()
                .clone();
            let _engine_guard = engine_lock.lock().await;
            let _file_guard = state.file_lock.lock().await;
            let (output_tx, mut output_rx) = tokio::sync::mpsc::unbounded_channel();
            let operation = execute(&state, &request, output_tx);
            tokio::pin!(operation);
            let outcome = loop {
                tokio::select! {
                    outcome = &mut operation => break outcome,
                    Some(line) = output_rx.recv() => {
                        running.output.push(line);
                        if running.output.len() > 500 { running.output.remove(0); }
                        running.progress = 50;
                        manager.publish(&running).await;
                    }
                }
            };
            match outcome {
                Ok(output) => {
                    running.state = JobState::Succeeded;
                    running.output = output.lines().map(str::to_owned).collect();
                }
                Err(error) => {
                    running.state = JobState::Failed;
                    running.error = Some(error.to_string());
                }
            }
            running.progress = 100;
            running.finished_at = Some(Utc::now());
            manager.publish(&running).await;
            let _ = state.refresh().await;
        });
        Ok(job)
    }
}

fn confirmed(options: &Value) -> anyhow::Result<()> {
    anyhow::ensure!(
        options["confirmed"] == true,
        "Confirmation explicite requise"
    );
    Ok(())
}
fn text<'a>(options: &'a Value, key: &str) -> anyhow::Result<&'a str> {
    options[key]
        .as_str()
        .filter(|s| !s.is_empty() && !s.starts_with('-'))
        .ok_or_else(|| anyhow::anyhow!("Champ {key} requis"))
}

async fn execute(
    state: &AppState,
    request: &StartJobRequest,
    output: tokio::sync::mpsc::UnboundedSender<String>,
) -> anyhow::Result<String> {
    let action = request.action.as_str();
    let options = &request.options;
    if action == "prune" {
        confirmed(options)?;
        let resources = options["resources"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Sélection requise"))?;
        anyhow::ensure!(resources.len() <= 100, "Maximum 100 ressources");
        let mut results = Vec::new();
        for resource in resources {
            let eligible = crate::api::preview(state).await?;
            anyhow::ensure!(
                eligible.as_array().unwrap().contains(resource),
                "Ressource devenue inéligible : {resource}"
            );
            let kind = text(resource, "kind")?;
            let id = text(resource, "id")?;
            let line = state
                .docker
                .run(&[kind.into(), "rm".into(), id.into()])
                .await?;
            let _ = output.send(format!("{kind} {id} : supprimé"));
            results.push(line);
        }
        return Ok(results.join("\n"));
    }
    state.refresh().await?;
    let inventory = state.inventory.read().await.clone();
    let probe_host =
        std::env::var("COMPMASTER_PROBE_HOST").unwrap_or_else(|_| "host.docker.internal".into());
    if let Some(project) = inventory.projects.iter().find(|p| p.id == request.target) {
        ensure_unique_project_name(&inventory, &project.id, &project.name)?;
        let definition = project.definition.as_ref().ok_or_else(|| {
            anyhow::anyhow!("Projet runtime uniquement : fichiers Compose non vérifiés")
        })?;
        anyhow::ensure!(definition.valid, "Compose invalide");
        if matches!(action, "down" | "stop" | "restart") {
            confirmed(options)?;
        }
        if matches!(action, "up" | "up_build" | "start" | "restart") {
            ports::preflight(&inventory.ports, &project.id, &probe_host).await?;
        }
        let path = state
            .scanner
            .resolve_definition_path(&project.id, &inventory)?;
        let mut args = vec![
            "compose".into(),
            "--project-directory".into(),
            path.parent().unwrap().to_string_lossy().into(),
            "-p".into(),
            project.name.clone(),
        ];
        for file in &definition.files {
            anyhow::ensure!(
                file.canonicalize()?.starts_with(&state.config.compose_root),
                "Fichier Compose hors racine"
            );
            args.extend(["-f".into(), file.to_string_lossy().into()]);
        }
        state
            .docker
            .run(&[args.clone(), vec!["config".into(), "--quiet".into()]].concat())
            .await?;
        match action {
            "up" => args.extend(["up".into(), "-d".into()]),
            "up_build" => args.extend(["up".into(), "--build".into(), "-d".into()]),
            "pull" | "build" | "start" | "stop" | "restart" | "down" => args.push(action.into()),
            _ => anyhow::bail!("Action non applicable au projet"),
        }
        return state.docker.run_reported(&args, output).await;
    }
    if matches!(action, "create_network" | "create_volume") {
        let name = text(options, "name")?;
        let kind = if action == "create_network" {
            "network"
        } else {
            "volume"
        };
        let mut args = vec![
            kind.into(),
            "create".into(),
            "--driver".into(),
            text(options, "driver")
                .unwrap_or(if kind == "network" { "bridge" } else { "local" })
                .into(),
        ];
        if kind == "network" {
            for (key, flag) in [("subnet", "--subnet"), ("gateway", "--gateway")] {
                if let Some(value) = options[key].as_str().filter(|s| !s.is_empty()) {
                    args.extend([flag.into(), value.into()]);
                }
            }
            for flag in ["internal", "ipv6", "attachable"] {
                if options[flag] == true {
                    args.push(format!("--{flag}"));
                }
            }
        }
        if let Some(driver_options) = options["driverOptions"].as_object() {
            for (key, value) in driver_options {
                let value = value
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Options du driver : chaînes attendues"))?;
                args.extend(["--opt".into(), format!("{key}={value}")]);
            }
        }
        args.push(name.into());
        return state.docker.run_reported(&args, output).await;
    }
    if let Some(id) = request.target.strip_prefix("image:") {
        anyhow::ensure!(
            inventory.images.iter().any(|i| i.id == id),
            "Image introuvable"
        );
        if action == "check_update" {
            let key = format!("image_update:{id}");
            if options["force"] != true {
                let cached = state.db.settings().await?;
                if let Some(value) = cached.get(&key) {
                    if value["checkedAt"]
                        .as_str()
                        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
                        .is_some_and(|t| (Utc::now() - t.with_timezone(&Utc)).num_hours() < 6)
                    {
                        return Ok(serde_json::to_string_pretty(value)?);
                    }
                }
            }
            let result = state.docker.image_update(id).await?;
            state.db.set(&key, &result).await?;
            return Ok(serde_json::to_string_pretty(&result)?);
        }
        anyhow::ensure!(action == "remove", "Action non applicable");
        confirmed(options)?;
        return state
            .docker
            .run(&["image".into(), "rm".into(), id.into()])
            .await;
    }
    for kind in ["network", "volume"] {
        if let Some(id) = request.target.strip_prefix(&format!("{kind}:")) {
            let exists = if kind == "network" {
                inventory.networks.iter().any(|n| n.id == id)
            } else {
                inventory.volumes.iter().any(|v| v.name == id)
            };
            anyhow::ensure!(exists, "Ressource introuvable");
            if matches!(action, "connect" | "disconnect") && kind == "network" {
                confirmed(options)?;
                let container = text(options, "container")?;
                ensure_container(&inventory, container)?;
                return state
                    .docker
                    .run(&["network".into(), action.into(), id.into(), container.into()])
                    .await;
            }
            anyhow::ensure!(action == "remove", "Action non applicable");
            confirmed(options)?;
            return state
                .docker
                .run(&[kind.into(), "rm".into(), id.into()])
                .await;
        }
    }
    ensure_container(&inventory, &request.target)?;
    if matches!(
        action,
        "remove" | "kill" | "stop" | "restart" | "rename" | "replace" | "pause" | "unpause"
    ) {
        confirmed(options)?;
    }
    if matches!(action, "start" | "restart" | "replace") {
        ports::preflight(&inventory.ports, &request.target, &probe_host).await?;
    }
    if action == "replace" {
        return replace_container(state, &request.target, options).await;
    }
    let mut args = vec![
        "container".into(),
        if action == "remove" { "rm" } else { action }.into(),
        request.target.clone(),
    ];
    if action == "rename" {
        args.push(text(options, "name")?.into());
    }
    anyhow::ensure!(
        matches!(
            action,
            "start" | "stop" | "restart" | "pause" | "unpause" | "kill" | "rename" | "remove"
        ),
        "Action non applicable"
    );
    state.docker.run_reported(&args, output).await
}

pub(crate) fn ensure_unique_project_name(
    inventory: &crate::models::Inventory,
    id: &str,
    name: &str,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        !inventory
            .projects
            .iter()
            .any(|project| project.id != id && project.name == name),
        "Nom de projet {name} ambigu : plusieurs définitions ou déploiements portent ce nom. Définissez des noms distincts dans les YAML avant toute action Compose."
    );
    Ok(())
}

pub fn ensure_container(inventory: &crate::models::Inventory, id: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        inventory
            .standalone
            .iter()
            .chain(inventory.projects.iter().flat_map(|p| &p.containers))
            .any(|c| c.id == id),
        "Conteneur introuvable"
    );
    Ok(())
}

async fn replace_container(state: &AppState, id: &str, options: &Value) -> anyhow::Result<String> {
    let old = state.docker.inspect("container", id).await?;
    anyhow::ensure!(
        old["Config"]["Labels"]["com.docker.compose.project"].is_null(),
        "Utilisez le YAML pour modifier un conteneur Compose"
    );
    anyhow::ensure!(
        old["HostConfig"]["AutoRemove"] != true,
        "Remplacement refusé : AutoRemove empêcherait le rollback"
    );
    let name = old["Name"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Nom absent"))?
        .trim_start_matches('/');
    let backup = format!(
        "{name}-cm-backup-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    let mut config = old["Config"].clone();
    let patch = options["config"]
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("config requise"))?;
    for (key, value) in patch {
        anyhow::ensure!(
            matches!(
                key.as_str(),
                "Image" | "Cmd" | "Entrypoint" | "Env" | "Healthcheck" | "WorkingDir" | "User"
            ),
            "Champ config non modifiable : {key}"
        );
        config[key] = value.clone();
    }
    let mut host = old["HostConfig"].clone();
    if let Some(patch) = options["hostConfig"].as_object() {
        for (key, value) in patch {
            anyhow::ensure!(
                matches!(
                    key.as_str(),
                    "PortBindings" | "Binds" | "Mounts" | "RestartPolicy" | "Memory" | "NanoCpus"
                ),
                "Champ hôte non modifiable : {key}"
            );
            host[key] = value.clone();
        }
    }
    // Preserve anonymous volumes too: Config.Volumes alone would allocate fresh storage.
    let mut mounts = host["Mounts"].as_array().cloned().unwrap_or_default();
    for mount in old["Mounts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["Type"] == "volume")
    {
        let destination = mount["Destination"].as_str().unwrap_or_default();
        let already_bound = mounts.iter().any(|m| m["Target"] == destination)
            || host["Binds"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .any(|bind| bind.split(':').nth(1) == Some(destination));
        if !already_bound {
            mounts.push(json!({"Type":"volume", "Source":mount["Name"], "Target":destination, "ReadOnly":mount["RW"] != true}));
        }
    }
    if !mounts.is_empty() {
        host["Mounts"] = mounts.into();
    }
    let inventory = state.inventory.read().await;
    let mut candidate_ports = inventory.ports.clone();
    if let Some(bindings) = host["PortBindings"].as_object() {
        for (target, entries) in bindings {
            let (container_port, protocol) = target
                .split_once('/')
                .ok_or_else(|| anyhow::anyhow!("Port cible invalide"))?;
            let container_port: u16 = container_port.parse()?;
            for (index, entry) in entries.as_array().into_iter().flatten().enumerate() {
                let raw = entry["HostPort"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("HostPort doit être une chaîne"))?;
                if raw.is_empty() || raw == "0" {
                    continue;
                }
                let host_port: u16 = raw.parse()?;
                candidate_ports.push(crate::models::PortBinding {
                    id: format!("replace:{target}:{index}"),
                    host_ip: entry["HostIp"].as_str().unwrap_or("0.0.0.0").into(),
                    host_port,
                    host_port_end: None,
                    container_port,
                    protocol: protocol.into(),
                    owner_id: id.into(),
                    owner_name: name.into(),
                    service: None,
                    source: crate::models::PortSource::Desired,
                    status: crate::models::PortStatus::Desired,
                    conflict: None,
                });
            }
        }
    }
    drop(inventory);
    ports::preflight(
        &candidate_ports,
        id,
        &std::env::var("COMPMASTER_PROBE_HOST").unwrap_or_else(|_| "host.docker.internal".into()),
    )
    .await?;
    // Deserialize before stopping anything, so malformed changes cannot cause downtime.
    let mut candidate_config = config.clone();
    candidate_config["HostConfig"] = host.clone();
    let _: bollard::models::ContainerCreateBody = serde_json::from_value(candidate_config)?;
    let client = state
        .docker
        .client
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Docker indisponible"))?;
    let was_running = old["State"]["Running"] == true;
    if was_running {
        state.docker.run(&["stop".into(), id.into()]).await?;
    }
    if let Err(error) = state
        .docker
        .run(&["rename".into(), id.into(), backup.clone()])
        .await
    {
        if was_running {
            let _ = state.docker.run(&["start".into(), id.into()]).await;
        }
        return Err(error);
    }
    config["HostConfig"] = host;
    let networks = old["NetworkSettings"]["Networks"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    let endpoints: serde_json::Map<String, Value> = networks
        .iter()
        .map(|(network, endpoint)| {
            (
                network.clone(),
                json!({"Aliases": endpoint["Aliases"], "IPAMConfig": endpoint["IPAMConfig"]}),
            )
        })
        .collect();
    config["NetworkingConfig"] = json!({"EndpointsConfig": endpoints});
    let mut created_id: Option<String> = None;
    let outcome: anyhow::Result<()> = async {
        let typed: bollard::models::ContainerCreateBody = serde_json::from_value(config)?;
        let created = client
            .create_container(
                Some(bollard::query_parameters::CreateContainerOptions {
                    name: Some(name.into()),
                    ..Default::default()
                }),
                typed,
            )
            .await?;
        created_id = Some(created.id.clone());
        state.docker.run(&["start".into(), created.id]).await?;
        for _ in 0..30 {
            let inspect = state.docker.inspect("container", name).await?;
            anyhow::ensure!(
                inspect["State"]["Running"] == true,
                "Le nouveau conteneur s'est arrêté"
            );
            match inspect["State"]["Health"]["Status"].as_str() {
                Some("healthy") => return Ok(()),
                Some("unhealthy") => anyhow::bail!("Healthcheck en échec"),
                None => {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    let inspect = state.docker.inspect("container", name).await?;
                    anyhow::ensure!(
                        inspect["State"]["Running"] == true,
                        "Le nouveau conteneur s'est arrêté"
                    );
                    return Ok(());
                }
                _ => tokio::time::sleep(std::time::Duration::from_secs(2)).await,
            }
        }
        anyhow::bail!("Healthcheck toujours en attente après 60 secondes")
    }
    .await;
    if let Err(error) = outcome {
        if let Some(created) = created_id {
            let _ = state
                .docker
                .run(&["container".into(), "rm".into(), "--force".into(), created])
                .await;
        }
        let restore = state
            .docker
            .run(&["rename".into(), id.into(), name.into()])
            .await;
        if was_running {
            let _ = state.docker.run(&["start".into(), id.into()]).await;
        }
        anyhow::bail!("Remplacement échoué : {error}. Restauration du nom : {restore:?}");
    }
    state
        .db
        .set(
            &format!("backup:{id}"),
            &json!({"name":backup,"replacement":name,"createdAt":Utc::now()}),
        )
        .await?;
    Ok(format!(
        "Remplacement validé. Ancien conteneur conservé sous {backup}. Supprimez-le séparément après vérification."
    ))
}
