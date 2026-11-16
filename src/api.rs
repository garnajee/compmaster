use crate::{AppState, models::StartJobRequest, security};
use axum::{
    Json, Router,
    extract::{
        DefaultBodyLimit, Path, Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode},
    middleware,
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
    routing::{get, post},
};
use futures_util::{SinkExt, Stream, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{convert::Infallible, path::PathBuf, sync::Arc, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tower_http::services::{ServeDir, ServeFile};

pub struct ApiError(StatusCode, String);
impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        Self(StatusCode::BAD_REQUEST, error.to_string())
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error": self.1}))).into_response()
    }
}
type Result<T> = std::result::Result<T, ApiError>;

macro_rules! ensure {
    ($condition:expr, $($arg:tt)*) => { if !$condition { return Err(anyhow::anyhow!($($arg)*).into()); } };
}

pub fn router(state: Arc<AppState>) -> Router {
    let static_dir = state.config.static_dir.clone();
    Router::new()
        .route("/api/v1/session", get(session))
        .route("/api/v1/inventory", get(inventory))
        .route("/api/v1/refresh", post(refresh))
        .route("/api/v1/events", get(events))
        .route("/api/v1/jobs", get(jobs).post(start_job))
        .route("/api/v1/settings", get(settings).put(save_settings))
        .route("/api/v1/projects/{id}/file", get(read_file).put(save_file))
        .route("/api/v1/projects/{id}/validate", post(validate_file))
        .route("/api/v1/inspect/{kind}/{id}", get(inspect))
        .route("/api/v1/containers/{id}/stats", get(stats))
        .route("/api/v1/containers/{id}/logs", get(logs))
        .route("/api/v1/ports/probe", post(probe_ports))
        .route("/api/v1/prune/preview", get(prune_preview))
        .route("/api/v1/prune", post(prune_selected))
        .route("/ws/terminal/{id}", get(terminal))
        .fallback_service(
            ServeDir::new(&static_dir)
                .not_found_service(ServeFile::new(static_dir.join("index.html"))),
        )
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            security::guard,
        ))
        .with_state(state)
}
async fn session(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(
        json!({"csrfToken": state.csrf_token, "root": state.config.compose_root, "version": env!("CARGO_PKG_VERSION"), "credentialDiagnostics": state.docker.credential_diagnostics().await}),
    )
}
async fn inventory(State(state): State<Arc<AppState>>) -> Json<Value> {
    let mut inventory = state.inventory.read().await.clone();
    let settings = state.db.settings().await.unwrap_or(json!({}));
    for project in &mut inventory.projects {
        project.favorite = settings[format!("favorite:{}", project.id)]
            .as_bool()
            .unwrap_or(false);
    }
    for image in &mut inventory.images {
        image.update_status = settings[format!("image_update:{}", image.id)]["status"]
            .as_str()
            .unwrap_or("unknown")
            .into();
    }
    Json(serde_json::to_value(inventory).unwrap_or(Value::Null))
}
async fn refresh(State(state): State<Arc<AppState>>) -> Result<Json<Value>> {
    state.refresh().await?;
    Ok(Json(json!({"ok": true})))
}
async fn settings(State(state): State<Arc<AppState>>) -> Result<Json<Value>> {
    Ok(Json(state.db.settings().await?))
}
async fn save_settings(
    State(state): State<Arc<AppState>>,
    Json(values): Json<Value>,
) -> Result<Json<Value>> {
    let values = values
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("Objet attendu"))?;
    for (key, value) in values {
        ensure!(
            key.starts_with("favorite:") || matches!(key.as_str(), "theme" | "filters"),
            "Préférence inconnue"
        );
        state.db.set(key, value).await?;
    }
    Ok(Json(json!({"ok": true})))
}
async fn jobs(State(state): State<Arc<AppState>>) -> Result<Json<Value>> {
    Ok(Json(
        serde_json::to_value(state.db.jobs().await?).map_err(anyhow::Error::from)?,
    ))
}
async fn start_job(
    State(state): State<Arc<AppState>>,
    Json(request): Json<StartJobRequest>,
) -> Result<Json<Value>> {
    Ok(Json(
        serde_json::to_value(state.jobs.submit(state.clone(), request).await?)
            .map_err(anyhow::Error::from)?,
    ))
}
async fn events(
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = std::result::Result<Event, Infallible>>> {
    let mut receiver = state.jobs.events.subscribe();
    Sse::new(async_stream::stream! {
        let mut tick = tokio::time::interval(Duration::from_secs(3));
        loop { tokio::select! {
            result = receiver.recv() => match result {
                Ok(job) => yield Ok(Event::default().event("job").json_data(job).unwrap()),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => yield Ok(Event::default().event("refresh").data("{}")),
                Err(_) => break,
            },
            _ = tick.tick() => { let generated = state.inventory.read().await.generated_at; yield Ok(Event::default().event("inventory").data(generated.to_rfc3339())); }
        } }
    }).keep_alive(KeepAlive::default())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileInput {
    content: String,
    revision: Option<String>,
}
#[derive(Default, Deserialize)]
struct FileQuery {
    #[serde(default)]
    index: usize,
}
async fn file_paths(
    state: &AppState,
    id: &str,
    index: usize,
) -> anyhow::Result<(PathBuf, Vec<PathBuf>)> {
    let inventory = state.inventory.read().await;
    let files = inventory
        .projects
        .iter()
        .find(|p| p.id == id)
        .and_then(|p| p.definition.as_ref())
        .ok_or_else(|| anyhow::anyhow!("Projet sans fichiers vérifiés"))?
        .files
        .clone();
    let path = files
        .get(index)
        .ok_or_else(|| anyhow::anyhow!("Fichier introuvable"))?
        .canonicalize()?;
    ensure!(
        path.starts_with(&state.config.compose_root),
        "Chemin hors racine"
    );
    Ok((path, files))
}
async fn read_file(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<FileQuery>,
) -> Result<Json<Value>> {
    let (path, files) = file_paths(&state, &id, query.index).await?;
    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(anyhow::Error::from)?;
    Ok(Json(
        json!({"content": content, "revision": hex::encode(Sha256::digest(content.as_bytes())), "path": path, "files": files}),
    ))
}
async fn validate_content(
    state: &AppState,
    path: &std::path::Path,
    files: &[PathBuf],
    content: &str,
) -> anyhow::Result<()> {
    let _: serde_yaml::Value = serde_yaml::from_str(content)?;
    let mut command = state.docker.command();
    command
        .args(["compose", "--project-directory"])
        .arg(files[0].parent().unwrap());
    for file in files {
        ensure!(
            file.canonicalize()?.starts_with(&state.config.compose_root),
            "Compose hors racine"
        );
        command.arg("-f");
        if file == path {
            command.arg("-");
        } else {
            command.arg(file);
        }
    }
    let mut child = command
        .args(["config", "--quiet"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(content.as_bytes())
        .await?;
    let output = tokio::time::timeout(Duration::from_secs(30), child.wait_with_output()).await??;
    ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}
async fn validate_file(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<FileQuery>,
    Json(input): Json<FileInput>,
) -> Result<Json<Value>> {
    let (path, files) = file_paths(&state, &id, query.index).await?;
    validate_content(&state, &path, &files, &input.content).await?;
    Ok(Json(json!({"valid": true})))
}
async fn save_file(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<FileQuery>,
    Json(input): Json<FileInput>,
) -> Result<Json<Value>> {
    let _lock = state.file_lock.lock().await;
    let (path, files) = file_paths(&state, &id, query.index).await?;
    let original = tokio::fs::read(&path).await.map_err(anyhow::Error::from)?;
    let revision = hex::encode(Sha256::digest(&original));
    if input.revision.as_deref() != Some(&revision) {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Le fichier a changé. Rechargez-le avant d'enregistrer.".into(),
        ));
    }
    validate_content(&state, &path, &files, &input.content).await?;
    // Recheck after external validation; do not overwrite edits made in an external editor.
    let current = tokio::fs::read(&path).await.map_err(anyhow::Error::from)?;
    if current != original {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Modification externe pendant la validation".into(),
        ));
    }
    let backup_id = hex::encode(Sha256::digest(id.as_bytes()));
    let backup = state.config.data_dir.join("backups").join(format!(
        "{}-{}.yaml",
        &backup_id[..16],
        uuid::Uuid::new_v4()
    ));
    tokio::fs::write(backup, &original)
        .await
        .map_err(anyhow::Error::from)?;
    let tmp = path.with_extension(format!("cm-{}.tmp", uuid::Uuid::new_v4()));
    let result: anyhow::Result<()> = async {
        let mut file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp)
            .await?;
        file.set_permissions(tokio::fs::metadata(&path).await?.permissions())
            .await?;
        file.write_all(input.content.as_bytes()).await?;
        file.sync_all().await?;
        ensure!(
            path.canonicalize()?.starts_with(&state.config.compose_root),
            "Chemin hors racine"
        );
        tokio::fs::rename(&tmp, &path).await?;
        Ok(())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
    }
    result?;
    state.refresh().await?;
    Ok(Json(
        json!({"ok": true, "revision": hex::encode(Sha256::digest(input.content.as_bytes()))}),
    ))
}
async fn inspect(
    State(state): State<Arc<AppState>>,
    Path((kind, id)): Path<(String, String)>,
) -> Result<Json<Value>> {
    ensure!(
        matches!(kind.as_str(), "container" | "image" | "network" | "volume"),
        "Type inconnu"
    );
    ensure!(!id.starts_with('-'), "Identifiant invalide");
    Ok(Json(state.docker.inspect(&kind, &id).await?))
}
async fn stats(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Result<Json<Value>> {
    crate::jobs::ensure_container(&*state.inventory.read().await, &id)?;
    Ok(Json(state.docker.stats(&id).await?))
}
async fn logs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Sse<impl Stream<Item = std::result::Result<Event, Infallible>>>> {
    crate::jobs::ensure_container(&*state.inventory.read().await, &id)?;
    let mut child = state.docker.logs(&id, "300")?;
    let mut stdout = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut stderr = BufReader::new(child.stderr.take().unwrap()).lines();
    Ok(Sse::new(async_stream::stream! {
        let mut out_open = true; let mut err_open = true;
        while out_open || err_open { tokio::select! {
            line = stdout.next_line(), if out_open => match line { Ok(Some(line)) => yield Ok(Event::default().event("log").data(json!({"stream":"stdout", "line":line}).to_string())), _ => out_open = false },
            line = stderr.next_line(), if err_open => match line { Ok(Some(line)) => yield Ok(Event::default().event("log").data(json!({"stream":"stderr", "line":line}).to_string())), _ => err_open = false },
        } }
        let _ = child.wait().await;
        yield Ok(Event::default().event("end").data("{}"));
    }).keep_alive(KeepAlive::default()))
}
#[derive(Deserialize)]
struct ProbeInput {
    ports: Vec<u16>,
}
async fn probe_ports(
    State(state): State<Arc<AppState>>,
    Json(input): Json<ProbeInput>,
) -> Result<Json<Value>> {
    ensure!(input.ports.len() <= 100, "Maximum 100 ports par requête");
    let host =
        std::env::var("COMPMASTER_PROBE_HOST").unwrap_or_else(|_| "host.docker.internal".into());
    let inventory = state.inventory.read().await.clone();
    let mut values = Vec::new();
    for port in input.ports {
        let status = if inventory.ports.iter().any(|p| {
            p.host_port == port
                && p.protocol == "tcp"
                && p.status == crate::models::PortStatus::DockerOccupied
        }) {
            "docker_occupied"
        } else {
            crate::ports::probe(&host, port).await
        };
        let mut suggested = crate::ports::suggestion(&inventory.ports, port);
        for _ in 0..20 {
            if let Some(next) = suggested {
                if crate::ports::probe(&host, next).await == "probably_free" {
                    break;
                }
                suggested = next.checked_add(1);
            }
        }
        values.push(json!({"port":port,"status":status,"suggestedPort":suggested}));
    }
    Ok(Json(
        json!({"host":host,"bestEffort":true,"results":values}),
    ))
}
pub(crate) async fn preview(state: &AppState) -> anyhow::Result<Value> {
    let settings = state.db.settings().await?;
    let mut result = Vec::new();
    for (kind, args) in [
        (
            "container",
            vec!["container", "ls", "-aq", "--filter", "status=exited"],
        ),
        (
            "image",
            vec!["image", "ls", "-q", "--filter", "dangling=true"],
        ),
        (
            "network",
            vec!["network", "ls", "-q", "--filter", "dangling=true"],
        ),
        (
            "volume",
            vec!["volume", "ls", "-q", "--filter", "dangling=true"],
        ),
    ] {
        let output = state
            .docker
            .run(&args.into_iter().map(str::to_owned).collect::<Vec<_>>())
            .await?;
        for id in output.lines().filter(|l| !l.is_empty()) {
            if kind == "container"
                && settings.as_object().is_some_and(|m| {
                    m.keys().any(|key| {
                        key.strip_prefix("backup:")
                            .is_some_and(|backup| backup.starts_with(id))
                    })
                })
            {
                continue;
            }
            result.push(json!({"kind":kind,"id":id}));
        }
    }
    Ok(result.into())
}
async fn prune_preview(State(state): State<Arc<AppState>>) -> Result<Json<Value>> {
    Ok(Json(preview(&state).await?))
}
#[derive(Deserialize, serde::Serialize)]
struct PruneInput {
    confirmed: bool,
    resources: Vec<PruneResource>,
}
#[derive(Deserialize, serde::Serialize)]
struct PruneResource {
    kind: String,
    id: String,
}
async fn prune_selected(
    State(state): State<Arc<AppState>>,
    Json(input): Json<PruneInput>,
) -> Result<Json<Value>> {
    ensure!(
        input.confirmed && input.resources.len() <= 100,
        "Confirmation requise, maximum 100 ressources"
    );
    let job = state
        .jobs
        .submit(
            state.clone(),
            StartJobRequest {
                target: "__prune__".into(),
                action: "prune".into(),
                options: serde_json::to_value(input).map_err(anyhow::Error::from)?,
            },
        )
        .await?;
    Ok(Json(
        serde_json::to_value(job).map_err(anyhow::Error::from)?,
    ))
}

#[derive(Deserialize)]
struct TerminalQuery {
    token: String,
    #[serde(default = "default_shell")]
    command: String,
}
fn default_shell() -> String {
    "/bin/sh".into()
}
async fn terminal(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<TerminalQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response> {
    ensure!(query.token == state.csrf_token, "Jeton invalide");
    ensure!(
        headers.get("origin").and_then(|h| h.to_str().ok())
            == Some(state.config.public_origin.as_str()),
        "Origin requise"
    );
    crate::jobs::ensure_container(&*state.inventory.read().await, &id)?;
    ensure!(
        !query.command.is_empty() && !query.command.starts_with('-') && query.command.len() < 1024,
        "Commande invalide"
    );
    Ok(ws.on_upgrade(move |socket| terminal_socket(state, id, query.command, socket)))
}
async fn terminal_socket(state: Arc<AppState>, id: String, command: String, mut socket: WebSocket) {
    let outcome: anyhow::Result<()> = async {
        let client = state.docker.client.as_ref().ok_or_else(|| anyhow::anyhow!("Docker indisponible"))?;
        let exec = client.create_exec(&id, bollard::exec::CreateExecOptions {
            attach_stdout: Some(true), attach_stderr: Some(true), attach_stdin: Some(true), tty: Some(true), cmd: Some(vec![command]), ..Default::default()
        }).await?;
        if let bollard::exec::StartExecResults::Attached { mut output, mut input } = client.start_exec(&exec.id, Some(bollard::exec::StartExecOptions { detach: false, tty: true, ..Default::default() })).await? {
            loop { tokio::select! {
                message = socket.recv() => match message {
                    Some(Ok(Message::Binary(bytes))) => input.write_all(&bytes).await?,
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(value) = serde_json::from_str::<Value>(&text) {
                            if let (Some(cols),Some(rows)) = (value["cols"].as_u64(),value["rows"].as_u64()) {
                                client.resize_exec(&exec.id, bollard::exec::ResizeExecOptions { width: cols.min(500) as u16, height: rows.min(200) as u16 }).await?;
                            }
                        }
                    },
                    Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => {},
                    _ => break,
                },
                chunk = output.next() => match chunk { Some(Ok(chunk)) => socket.send(Message::Binary(chunk.into_bytes())).await?, Some(Err(error)) => return Err(error.into()), None => break },
            } }
        }
        Ok(())
    }.await;
    if let Err(error) = outcome {
        let _ = socket
            .send(Message::Text(format!("\r\n{error}\r\n").into()))
            .await;
    }
    let _ = socket.close().await;
}
