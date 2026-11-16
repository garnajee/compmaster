use std::{env, net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result, bail};

#[derive(Clone, Debug)]
pub struct Config {
    pub listen: SocketAddr,
    pub compose_root: PathBuf,
    pub data_dir: PathBuf,
    pub database_url: String,
    pub docker_host: String,
    pub docker_cli: PathBuf,
    pub static_dir: PathBuf,
    pub public_origin: String,
    pub scan_concurrency: usize,
    pub max_job_history: usize,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let listen = env::var("COMPMASTER_LISTEN")
            .unwrap_or_else(|_| "127.0.0.1:8080".into())
            .parse()
            .context("COMPMASTER_LISTEN must be a socket address")?;
        let compose_root = absolute_dir(env::var("COMPOSE_ROOT").unwrap_or_else(|_| ".".into()))?;
        let data_dir =
            PathBuf::from(env::var("COMPMASTER_DATA_DIR").unwrap_or_else(|_| "./data".into()));
        std::fs::create_dir_all(&data_dir)
            .with_context(|| format!("unable to create {}", data_dir.display()))?;
        std::fs::create_dir_all(data_dir.join("backups"))?;

        Ok(Self {
            listen,
            compose_root,
            database_url: format!(
                "sqlite://{}?mode=rwc",
                data_dir.join("compmaster.db").display()
            ),
            data_dir,
            docker_host: env::var("DOCKER_HOST")
                .unwrap_or_else(|_| "unix:///var/run/docker.sock".into()),
            docker_cli: PathBuf::from(env::var("DOCKER_CLI").unwrap_or_else(|_| "docker".into())),
            static_dir: PathBuf::from(
                env::var("COMPMASTER_STATIC_DIR").unwrap_or_else(|_| "./frontend/dist".into()),
            ),
            public_origin: env::var("COMPMASTER_PUBLIC_ORIGIN")
                .unwrap_or_else(|_| "http://127.0.0.1:8080".into()),
            scan_concurrency: env::var("COMPMASTER_SCAN_CONCURRENCY")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(4),
            max_job_history: 500,
        })
    }
}

fn absolute_dir(value: String) -> Result<PathBuf> {
    let path = PathBuf::from(value);
    let path = if path.is_absolute() {
        path
    } else {
        env::current_dir()?.join(path)
    };
    if !path.is_dir() {
        bail!("COMPOSE_ROOT is not a directory: {}", path.display());
    }
    Ok(path.canonicalize()?)
}
