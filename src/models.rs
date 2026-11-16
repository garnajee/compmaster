use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{config::Config, docker::RuntimeSnapshot, ports};

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inventory {
    pub generated_at: DateTime<Utc>,
    pub engine_connected: bool,
    pub engine_error: Option<String>,
    pub projects: Vec<ComposeDeployment>,
    pub standalone: Vec<StandaloneContainer>,
    pub images: Vec<ImageSummary>,
    pub networks: Vec<NetworkSummary>,
    pub volumes: Vec<VolumeSummary>,
    pub ports: Vec<PortBinding>,
    pub summary: InventorySummary,
    pub events: Vec<serde_json::Value>,
}

impl Inventory {
    pub async fn merge(
        definitions: Vec<ComposeDefinition>,
        runtime: RuntimeSnapshot,
        config: &Config,
    ) -> Self {
        let mut projects: BTreeMap<String, ComposeDeployment> = BTreeMap::new();
        let mut standalone = Vec::new();

        for container in runtime.containers.iter().cloned() {
            if let Some(project_name) = container.labels.get("com.docker.compose.project").cloned()
            {
                let service = container.labels.get("com.docker.compose.service").cloned();
                let project = projects
                    .entry(format!("runtime:{project_name}"))
                    .or_insert_with(|| ComposeDeployment {
                        id: format!("runtime:{project_name}"),
                        name: project_name,
                        state: ResourceState::RuntimeOnly,
                        definition: None,
                        containers: Vec::new(),
                        services: Vec::new(),
                        favorite: false,
                        diagnostics: Vec::new(),
                    });
                if let Some(service) = service {
                    if !project.services.contains(&service) {
                        project.services.push(service);
                    }
                }
                project.containers.push(container);
            } else {
                standalone.push(container);
            }
        }

        let mut consumed = HashSet::new();
        for project in projects.values_mut() {
            // All containers must agree on ordered config files. Hints alone never authorize an action.
            let hints = runtime_file_hints(project, &config.compose_root);
            let mut matched = None;
            if let Some(paths) = hints {
                if let Ok(definition) =
                    crate::scanner::validate_deployment(config, &project.name, &paths).await
                {
                    let actual: HashSet<_> = project.services.iter().collect();
                    let expected: HashSet<_> = definition.services.iter().collect();
                    if !actual.is_empty() && actual.is_subset(&expected) {
                        matched = Some(definition);
                    }
                }
            } else if project.containers.iter().all(|container| {
                !container
                    .labels
                    .contains_key("com.docker.compose.project.config_files")
            }) {
                let candidates: Vec<_> = definitions
                    .iter()
                    .filter(|d| {
                        d.valid && d.explicit_name && d.project_name == project.name && {
                            let actual: HashSet<_> = project.services.iter().collect();
                            let expected: HashSet<_> = d.services.iter().collect();
                            !actual.is_empty() && actual == expected
                        }
                    })
                    .collect();
                if candidates.len() == 1 {
                    matched = Some(candidates[0].clone());
                }
            }
            if let Some(definition) = matched {
                consumed.extend(definition.files.iter().cloned());
                project.id = if definition.files.len() == 1
                    && definitions
                        .iter()
                        .any(|d| d.path == definition.path && d.project_name == project.name)
                {
                    definition.id.clone()
                } else {
                    format!("deployment:{}:{}", project.name, definition.id)
                };
                let running = project
                    .containers
                    .iter()
                    .filter(|c| c.state == "running")
                    .count();
                project.state = if running == project.containers.len() {
                    ResourceState::Running
                } else if running == 0 {
                    ResourceState::Stopped
                } else {
                    ResourceState::Partial
                };
                project.definition = Some(definition);
            }
        }
        for definition in definitions {
            if !consumed.contains(&definition.path) {
                projects.insert(
                    definition.id.clone(),
                    ComposeDeployment {
                        id: definition.id.clone(),
                        name: definition.project_name.clone(),
                        state: if definition.valid {
                            ResourceState::Discovered
                        } else {
                            ResourceState::Invalid
                        },
                        services: definition.services.clone(),
                        definition: Some(definition.clone()),
                        containers: Vec::new(),
                        favorite: false,
                        diagnostics: definition.diagnostics.clone(),
                    },
                );
            }
        }

        let mut ports = ports::from_inventory(projects.values(), &standalone);
        ports::mark_conflicts(&mut ports);
        let projects: Vec<_> = projects.into_values().collect();
        let summary = InventorySummary {
            projects: projects.len(),
            running: projects
                .iter()
                .filter(|p| p.containers.iter().any(|c| c.state == "running"))
                .count(),
            standalone: standalone.len(),
            unhealthy: projects
                .iter()
                .flat_map(|p| &p.containers)
                .filter(|c| c.health.as_deref() == Some("unhealthy"))
                .count(),
            port_conflicts: ports.iter().filter(|p| p.conflict.is_some()).count(),
        };

        Self {
            generated_at: Utc::now(),
            engine_connected: runtime.connected,
            engine_error: runtime.error,
            projects,
            standalone,
            images: runtime.images,
            networks: runtime.networks,
            volumes: runtime.volumes,
            ports,
            summary,
            events: Vec::new(),
        }
    }
}

fn runtime_file_hints(project: &ComposeDeployment, root: &std::path::Path) -> Option<Vec<PathBuf>> {
    let mut agreed: Option<Vec<PathBuf>> = None;
    for container in &project.containers {
        let label = container
            .labels
            .get("com.docker.compose.project.config_files")?;
        let paths: Vec<_> = label
            .split(',')
            .map(|path| std::fs::canonicalize(path.trim()))
            .collect::<std::io::Result<_>>()
            .ok()?;
        if paths.is_empty() || paths.iter().any(|p| !p.starts_with(root)) {
            return None;
        }
        if agreed.as_ref().is_some_and(|previous| previous != &paths) {
            return None;
        }
        agreed = Some(paths);
    }
    agreed
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeDefinition {
    pub id: String,
    pub path: PathBuf,
    pub files: Vec<PathBuf>,
    pub explicit_name: bool,
    pub relative_path: PathBuf,
    pub project_name: String,
    pub services: Vec<String>,
    pub desired_ports: Vec<DesiredPort>,
    pub valid: bool,
    pub revision: String,
    pub diagnostics: Vec<ValidationDiagnostic>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeDeployment {
    pub id: String,
    pub name: String,
    pub state: ResourceState,
    pub definition: Option<ComposeDefinition>,
    pub containers: Vec<StandaloneContainer>,
    pub services: Vec<String>,
    pub favorite: bool,
    pub diagnostics: Vec<ValidationDiagnostic>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StandaloneContainer {
    pub id: String,
    pub short_id: String,
    pub name: String,
    pub image: String,
    pub image_id: String,
    pub state: String,
    pub status: String,
    pub health: Option<String>,
    pub created: i64,
    pub labels: HashMap<String, String>,
    pub ports: Vec<ContainerPort>,
    pub mounts: Vec<MountSummary>,
    pub networks: Vec<String>,
    pub network_addresses: HashMap<String, Vec<String>>,
    pub provenance: Option<Provenance>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
    pub kind: String,
    pub source: String,
    pub evidence: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerPort {
    pub host_ip: String,
    pub host_port: u16,
    pub container_port: u16,
    pub protocol: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesiredPort {
    pub service: String,
    pub host_ip: String,
    pub host_port: u16,
    pub host_port_end: Option<u16>,
    pub container_port: u16,
    pub protocol: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortBinding {
    pub id: String,
    pub host_ip: String,
    pub host_port: u16,
    pub host_port_end: Option<u16>,
    pub container_port: u16,
    pub protocol: String,
    pub owner_id: String,
    pub owner_name: String,
    pub service: Option<String>,
    pub source: PortSource,
    pub status: PortStatus,
    pub conflict: Option<PortConflict>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortConflict {
    pub kind: String,
    pub message: String,
    pub conflicting_owner: Option<String>,
    pub suggested_port: Option<u16>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PortSource {
    Runtime,
    Desired,
    External,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PortStatus {
    DockerOccupied,
    StoppedReservation,
    Desired,
    ExternalDetected,
    ProbablyFree,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceState {
    Discovered,
    Running,
    Stopped,
    Partial,
    Invalid,
    RuntimeOnly,
}

#[derive(Clone, Debug, Serialize)]
pub struct ValidationDiagnostic {
    pub level: DiagnosticLevel,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticLevel {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MountSummary {
    pub kind: String,
    pub source: String,
    pub destination: String,
    pub read_only: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageSummary {
    pub id: String,
    pub tags: Vec<String>,
    pub size: i64,
    pub created: i64,
    pub containers: i64,
    pub update_status: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkSummary {
    pub id: String,
    pub name: String,
    pub driver: String,
    pub scope: String,
    pub internal: bool,
    pub containers: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VolumeSummary {
    pub name: String,
    pub driver: String,
    pub mountpoint: String,
    pub labels: HashMap<String, String>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventorySummary {
    pub projects: usize,
    pub running: usize,
    pub standalone: usize,
    pub unhealthy: usize,
    pub port_conflicts: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub target: String,
    pub action: String,
    pub state: JobState,
    pub progress: u8,
    pub output: Vec<String>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartJobRequest {
    pub target: String,
    pub action: String,
    #[serde(default)]
    pub options: serde_json::Value,
}
