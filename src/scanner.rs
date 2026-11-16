use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use futures_util::{StreamExt, stream};
use ignore::WalkBuilder;
use serde_json::Value as JsonValue;
use serde_yaml::Value as YamlValue;
use sha2::{Digest, Sha256};
use tokio::{process::Command, sync::Semaphore};

use crate::models::{ComposeDefinition, DesiredPort, DiagnosticLevel, ValidationDiagnostic};

#[derive(Clone)]
pub struct ComposeScanner {
    root: PathBuf,
    docker_cli: PathBuf,
    concurrency: usize,
}

impl ComposeScanner {
    pub fn new(root: PathBuf, docker_cli: PathBuf, concurrency: usize) -> Self {
        Self {
            root,
            docker_cli,
            concurrency: concurrency.max(1),
        }
    }

    pub async fn scan(&self) -> Vec<ComposeDefinition> {
        let root = self.root.clone();
        let paths = tokio::task::spawn_blocking(move || candidate_paths(&root))
            .await
            .unwrap_or_default();
        let semaphore = Arc::new(Semaphore::new(self.concurrency));
        let this = self.clone();
        let mut definitions: Vec<_> = stream::iter(paths)
            .map(|path| {
                let scanner = this.clone();
                let semaphore = semaphore.clone();
                async move {
                    let _permit = semaphore.acquire().await.ok()?;
                    scanner.inspect(path).await
                }
            })
            .buffer_unordered(self.concurrency)
            .filter_map(std::future::ready)
            .collect()
            .await;
        definitions.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
        definitions
    }

    async fn inspect(&self, path: PathBuf) -> Option<ComposeDefinition> {
        let content = tokio::fs::read_to_string(&path).await.ok()?;
        let revision = hex::encode(Sha256::digest(content.as_bytes()));
        let relative_path = path.strip_prefix(&self.root).ok()?.to_path_buf();
        let mut diagnostics = Vec::new();
        let yaml: YamlValue = match serde_yaml::from_str(&content) {
            Ok(value) => value,
            Err(error) => {
                return Some(self.invalid_definition(
                    path,
                    relative_path,
                    revision,
                    error.to_string(),
                ));
            }
        };

        let mut services = yaml_services(&yaml);
        if yaml_get(&yaml, "services").is_none() && yaml_get(&yaml, "include").is_none() {
            return None;
        }
        let explicit_name = yaml_get(&yaml, "name")
            .and_then(YamlValue::as_str)
            .map(str::to_owned);
        let fallback_name = stable_project_name(&relative_path);
        let mut project_name = explicit_name
            .clone()
            .unwrap_or_else(|| fallback_name.clone());
        let mut desired_ports = yaml_ports(&yaml);

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            Command::new(&self.docker_cli)
                .arg("compose")
                .arg("--project-directory")
                .arg(path.parent().unwrap_or(&self.root))
                .arg("-f")
                .arg(&path)
                .args(["config", "--format", "json"])
                .env("COMPOSE_ANSI", "never")
                .kill_on_drop(true)
                .output(),
        )
        .await;

        let valid = match output {
            Ok(Ok(output)) if output.status.success() => {
                if let Ok(canonical) = serde_json::from_slice::<JsonValue>(&output.stdout) {
                    if explicit_name.is_some() {
                        if let Some(name) = canonical["name"].as_str() {
                            project_name = name.to_owned();
                        }
                    }
                    desired_ports = canonical_ports(&canonical);
                    services = canonical["services"]
                        .as_object()
                        .map(|s| s.keys().cloned().collect())
                        .unwrap_or_default();
                }
                true
            }
            Ok(Ok(output)) => {
                diagnostics.push(ValidationDiagnostic {
                    level: DiagnosticLevel::Error,
                    message: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
                });
                false
            }
            error => {
                diagnostics.push(ValidationDiagnostic {
                    level: DiagnosticLevel::Warning,
                    message: format!("Validation Compose impossible : {error:?}"),
                });
                false
            }
        };

        Some(ComposeDefinition {
            id: definition_id(&relative_path),
            files: vec![path.clone()],
            explicit_name: explicit_name.is_some(),
            path,
            relative_path,
            project_name,
            services,
            desired_ports,
            valid,
            revision,
            diagnostics,
        })
    }

    fn invalid_definition(
        &self,
        path: PathBuf,
        relative_path: PathBuf,
        revision: String,
        message: String,
    ) -> ComposeDefinition {
        ComposeDefinition {
            id: definition_id(&relative_path),
            files: vec![path.clone()],
            explicit_name: false,
            path,
            project_name: stable_project_name(&relative_path),
            relative_path,
            services: Vec::new(),
            desired_ports: Vec::new(),
            valid: false,
            revision,
            diagnostics: vec![ValidationDiagnostic {
                level: DiagnosticLevel::Error,
                message,
            }],
        }
    }

    pub fn resolve_definition_path(
        &self,
        id: &str,
        inventory: &crate::models::Inventory,
    ) -> anyhow::Result<PathBuf> {
        let path = inventory
            .projects
            .iter()
            .find(|project| project.id == id)
            .and_then(|project| project.definition.as_ref())
            .map(|definition| definition.path.clone())
            .ok_or_else(|| anyhow::anyhow!("définition Compose introuvable"))?;
        let canonical = path.canonicalize()?;
        anyhow::ensure!(
            canonical.starts_with(&self.root),
            "chemin hors de COMPOSE_ROOT"
        );
        Ok(canonical)
    }
}

fn candidate_paths(root: &Path) -> Vec<PathBuf> {
    let mut builder = WalkBuilder::new(root);
    builder
        .follow_links(false)
        .hidden(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .add_custom_ignore_filename(".compmasterignore")
        .filter_entry(|entry| entry.file_name() != ".git");
    builder
        .build()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
        .map(|entry| entry.into_path())
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| matches!(ext.to_ascii_lowercase().as_str(), "yml" | "yaml"))
        })
        .collect()
}

fn yaml_get<'a>(value: &'a YamlValue, key: &str) -> Option<&'a YamlValue> {
    value.as_mapping()?.get(YamlValue::String(key.into()))
}

fn yaml_services(value: &YamlValue) -> Vec<String> {
    yaml_get(value, "services")
        .and_then(YamlValue::as_mapping)
        .map(|services| {
            services
                .keys()
                .filter_map(YamlValue::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn yaml_ports(value: &YamlValue) -> Vec<DesiredPort> {
    let Some(services) = yaml_get(value, "services").and_then(YamlValue::as_mapping) else {
        return Vec::new();
    };
    let mut ports = Vec::new();
    for (service_name, service) in services {
        let Some(service_name) = service_name.as_str() else {
            continue;
        };
        let Some(items) = yaml_get(service, "ports").and_then(YamlValue::as_sequence) else {
            continue;
        };
        for item in items {
            if let Some(port) = parse_yaml_port(service_name, item) {
                ports.push(port);
            }
        }
    }
    ports
}

fn parse_yaml_port(service: &str, value: &YamlValue) -> Option<DesiredPort> {
    if let Some(raw) = value.as_str() {
        let (raw, protocol) = raw
            .rsplit_once('/')
            .map_or((raw, "tcp"), |(p, proto)| (p, proto));
        let segments: Vec<_> = raw.rsplitn(3, ':').collect();
        if segments.len() < 2 {
            return None;
        }
        let container_port = segments[0].parse().ok()?;
        let host_port = segments[1].parse().ok()?;
        let host_ip = if segments.len() == 3 {
            segments[2].trim_matches(['[', ']']).to_owned()
        } else {
            "0.0.0.0".into()
        };
        return Some(DesiredPort {
            service: service.into(),
            host_ip,
            host_port,
            host_port_end: None,
            container_port,
            protocol: protocol.into(),
        });
    }
    let mapping = value.as_mapping()?;
    let published = mapping.get(YamlValue::String("published".into()))?;
    let host_port = published
        .as_u64()
        .or_else(|| published.as_str()?.parse().ok())? as u16;
    let target = mapping.get(YamlValue::String("target".into()))?;
    let container_port = target.as_u64().or_else(|| target.as_str()?.parse().ok())? as u16;
    let host_ip = mapping
        .get(YamlValue::String("host_ip".into()))
        .and_then(YamlValue::as_str)
        .unwrap_or("0.0.0.0")
        .into();
    let protocol = mapping
        .get(YamlValue::String("protocol".into()))
        .and_then(YamlValue::as_str)
        .unwrap_or("tcp")
        .into();
    Some(DesiredPort {
        service: service.into(),
        host_ip,
        host_port,
        host_port_end: None,
        container_port,
        protocol,
    })
}

pub fn canonical_ports(value: &JsonValue) -> Vec<DesiredPort> {
    let mut ports = Vec::new();
    let Some(services) = value.get("services").and_then(JsonValue::as_object) else {
        return ports;
    };
    for (service_name, service) in services {
        let Some(items) = service.get("ports").and_then(JsonValue::as_array) else {
            continue;
        };
        for item in items {
            let Some(published) = item.get("published") else {
                continue;
            };
            let raw = published
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| published.to_string());
            let (start, end) = raw
                .split_once('-')
                .map_or((raw.as_str(), None), |(a, b)| (a, b.parse::<u16>().ok()));
            let Ok(host_port) = start.parse::<u16>() else {
                continue;
            };
            if host_port == 0 {
                continue;
            } // Ephemeral publication: actual port is discovered from Docker after start.
            let Some(container_port) = item.get("target").and_then(JsonValue::as_u64) else {
                continue;
            };
            ports.push(DesiredPort {
                service: service_name.clone(),
                host_ip: item
                    .get("host_ip")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("0.0.0.0")
                    .into(),
                host_port,
                host_port_end: end.filter(|end| *end >= host_port),
                container_port: container_port as u16,
                protocol: item
                    .get("protocol")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("tcp")
                    .into(),
            });
        }
    }
    ports
}

pub fn definition_id(relative: &Path) -> String {
    let digest = Sha256::digest(relative.to_string_lossy().as_bytes());
    format!("cmp_{}", &hex::encode(digest)[..16])
}

pub async fn validate_deployment(
    config: &crate::config::Config,
    name: &str,
    paths: &[PathBuf],
) -> anyhow::Result<ComposeDefinition> {
    anyhow::ensure!(!paths.is_empty(), "Aucun fichier Compose");
    let mut command = Command::new(&config.docker_cli);
    command
        .args(["compose", "--project-directory"])
        .arg(paths[0].parent().unwrap())
        .args(["-p", name])
        .env("DOCKER_HOST", &config.docker_host)
        .kill_on_drop(true);
    for path in paths {
        anyhow::ensure!(
            path.canonicalize()?.starts_with(&config.compose_root),
            "Compose hors racine"
        );
        command.arg("-f").arg(path);
    }
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        command.args(["config", "--format", "json"]).output(),
    )
    .await??;
    anyhow::ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let canonical: JsonValue = serde_json::from_slice(&output.stdout)?;
    let content = tokio::fs::read(&paths[0]).await?;
    let relative_path = paths[0].strip_prefix(&config.compose_root)?.to_path_buf();
    Ok(ComposeDefinition {
        id: definition_id(&relative_path),
        path: paths[0].clone(),
        files: paths.to_vec(),
        explicit_name: true,
        relative_path,
        project_name: name.into(),
        services: canonical["services"]
            .as_object()
            .map(|s| s.keys().cloned().collect())
            .unwrap_or_default(),
        desired_ports: canonical_ports(&canonical),
        valid: true,
        revision: hex::encode(Sha256::digest(content)),
        diagnostics: Vec::new(),
    })
}

fn stable_project_name(relative: &Path) -> String {
    let stem = relative
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("compose");
    let parent = relative
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .unwrap_or("root");
    let slug = format!("{parent}-{stem}")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_owned();
    format!(
        "cm-{}-{}",
        &slug[..slug.len().min(38)],
        &definition_id(relative)[4..12]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_short_and_long_ports() {
        let yaml: YamlValue = serde_yaml::from_str(
            r#"
services:
  api:
    ports:
      - "127.0.0.1:8000:80/tcp"
      - target: 443
        published: 8443
        protocol: tcp
"#,
        )
        .unwrap();
        let ports = yaml_ports(&yaml);
        assert_eq!(ports.len(), 2);
        assert_eq!(ports[0].host_port, 8000);
        assert_eq!(ports[1].container_port, 443);
    }

    #[test]
    fn stable_ids_depend_on_relative_path() {
        assert_eq!(
            definition_id(Path::new("a/compose.yml")),
            definition_id(Path::new("a/compose.yml"))
        );
        assert_ne!(
            definition_id(Path::new("a/compose.yml")),
            definition_id(Path::new("b/compose.yml"))
        );
    }
}
