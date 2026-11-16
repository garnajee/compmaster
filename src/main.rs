mod api;
mod config;
mod db;
mod docker;
mod jobs;
mod models;
mod ports;
mod scanner;
mod security;
#[cfg(test)]
mod tests;

use std::{sync::Arc, time::Duration};

use anyhow::Context;
use tokio::sync::RwLock;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::{
    config::Config, db::Database, docker::DockerService, jobs::JobManager, models::Inventory,
    scanner::ComposeScanner,
};

pub struct AppState {
    pub config: Config,
    pub db: Database,
    pub docker: DockerService,
    pub scanner: ComposeScanner,
    pub jobs: JobManager,
    pub inventory: RwLock<Inventory>,
    pub csrf_token: String,
    pub file_lock: tokio::sync::Mutex<()>,
    pub refresh_lock: tokio::sync::Mutex<()>,
    pub docker_events: RwLock<Vec<serde_json::Value>>,
}

impl AppState {
    async fn refresh(&self) -> anyhow::Result<()> {
        let _guard = self.refresh_lock.lock().await;
        let definitions = self.scanner.scan().await;
        let runtime = self.docker.snapshot().await;
        let mut inventory = models::Inventory::merge(definitions, runtime, &self.config).await;
        ports::discover_external(
            &mut inventory.ports,
            &std::env::var("COMPMASTER_PROBE_HOST")
                .unwrap_or_else(|_| "host.docker.internal".into()),
        )
        .await;
        inventory.summary.port_conflicts = inventory
            .ports
            .iter()
            .filter(|p| p.conflict.is_some())
            .count();
        inventory.events = self.docker_events.read().await.clone();
        *self.inventory.write().await = inventory;
        Ok(())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::args().any(|arg| arg == "--healthcheck") {
        use std::io::{Read, Write};
        let address =
            std::env::var("COMPMASTER_LISTEN").unwrap_or_else(|_| "127.0.0.1:8080".into());
        let port = address.rsplit(':').next().unwrap_or("8080");
        let mut stream = std::net::TcpStream::connect(format!("127.0.0.1:{port}"))?;
        stream.set_read_timeout(Some(Duration::from_secs(3)))?;
        let origin = std::env::var("COMPMASTER_PUBLIC_ORIGIN")
            .unwrap_or_else(|_| "http://127.0.0.1:8080".into());
        let authority = origin
            .split_once("://")
            .map(|(_, h)| h)
            .unwrap_or("127.0.0.1:8080");
        write!(
            stream,
            "GET /api/v1/session HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n"
        )?;
        let mut buffer = [0u8; 64];
        let length = stream.read(&mut buffer)?;
        anyhow::ensure!(
            String::from_utf8_lossy(&buffer[..length]).starts_with("HTTP/1.1 200"),
            "Healthcheck failed"
        );
        return Ok(());
    }
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "compmaster=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = Config::from_env()?;
    let db = Database::connect(&config.database_url).await?;
    let docker = DockerService::connect(&config.docker_host, config.docker_cli.clone());
    let scanner = ComposeScanner::new(
        config.compose_root.clone(),
        config.docker_cli.clone(),
        config.scan_concurrency,
    );
    let jobs = JobManager::new(db.clone(), config.max_job_history);

    let state = Arc::new(AppState {
        config: config.clone(),
        db,
        docker,
        scanner,
        jobs,
        inventory: RwLock::new(Inventory::default()),
        csrf_token: security::new_csrf_token(),
        file_lock: Default::default(),
        refresh_lock: Default::default(),
        docker_events: Default::default(),
    });

    if let Err(error) = state.refresh().await {
        error!(%error, "initial inventory failed");
    }

    let (refresh_tx, mut refresh_rx) = tokio::sync::mpsc::channel(1);
    let docker = state.docker.clone();
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(128);
    tokio::spawn(async move {
        docker.watch_events(event_tx).await;
    });
    let event_state = state.clone();
    let docker_refresh_tx = refresh_tx.clone();
    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            let mut events = event_state.docker_events.write().await;
            events.insert(0, event);
            events.truncate(200);
            drop(events);
            let _ = docker_refresh_tx.try_send(());
        }
    });
    use notify::Watcher;
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        if let Ok(event) = result {
            if !matches!(event.kind, notify::EventKind::Access(_))
                && event.paths.iter().any(|path| {
                    path.extension()
                        .is_some_and(|ext| ext == "yaml" || ext == "yml" || ext == "env")
                        || path
                            .file_name()
                            .is_some_and(|name| name == ".env" || name == ".compmasterignore")
                })
            {
                let _ = refresh_tx.try_send(());
            }
        }
    })?;
    watcher.watch(&config.compose_root, notify::RecursiveMode::Recursive)?;
    let event_state = state.clone();
    tokio::spawn(async move {
        while refresh_rx.recv().await.is_some() {
            tokio::time::sleep(Duration::from_millis(500)).await;
            while refresh_rx.try_recv().is_ok() {}
            if let Err(error) = event_state.refresh().await {
                error!(%error, "event refresh failed");
            }
        }
    });

    let refresh_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(error) = refresh_state.refresh().await {
                error!(%error, "inventory refresh failed");
            }
        }
    });

    let app = api::router(state);
    let listener = tokio::net::TcpListener::bind(config.listen)
        .await
        .with_context(|| format!("unable to bind {}", config.listen))?;
    info!(address = %config.listen, root = %config.compose_root.display(), "CompMaster ready");
    axum::serve(listener, app).await?;
    Ok(())
}
