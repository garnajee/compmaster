use crate::models::*;
use bollard::Docker;
use serde_json::{Value, json};
use std::{collections::HashMap, path::PathBuf};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};

#[derive(Clone)]
pub struct DockerService {
    pub client: Option<Docker>,
    pub cli: PathBuf,
    pub host: String,
}

#[derive(Default)]
pub struct RuntimeSnapshot {
    pub connected: bool,
    pub error: Option<String>,
    pub containers: Vec<StandaloneContainer>,
    pub images: Vec<ImageSummary>,
    pub networks: Vec<NetworkSummary>,
    pub volumes: Vec<VolumeSummary>,
}

impl DockerService {
    pub fn connect(host: &str, cli: PathBuf) -> Self {
        let client = if let Some(path) = host.strip_prefix("unix://") {
            Docker::connect_with_unix(path, 15, bollard::API_DEFAULT_VERSION).ok()
        } else {
            None
        };
        Self {
            client,
            cli,
            host: host.into(),
        }
    }
    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.cli);
        command
            .env("DOCKER_HOST", &self.host)
            .env("COMPOSE_ANSI", "never")
            .kill_on_drop(true);
        command
    }
    pub async fn snapshot(&self) -> RuntimeSnapshot {
        match self.try_snapshot().await {
            Ok(snapshot) => snapshot,
            Err(error) => RuntimeSnapshot {
                error: Some(error.to_string()),
                ..Default::default()
            },
        }
    }
    async fn try_snapshot(&self) -> anyhow::Result<RuntimeSnapshot> {
        let base_client = self
            .client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Socket Unix Docker requis ; vérifiez DOCKER_HOST"))?;
        let negotiated = base_client.clone().negotiate_version().await?;
        let client = &negotiated;
        client.ping().await?;
        let containers = client
            .list_containers(Some(bollard::query_parameters::ListContainersOptions {
                all: true,
                ..Default::default()
            }))
            .await?;
        let mut result = RuntimeSnapshot {
            connected: true,
            ..Default::default()
        };
        for container in containers {
            let value = serde_json::to_value(container)?;
            let id = string(&value, "Id");
            let inspect = client
                .inspect_container(
                    &id,
                    None::<bollard::query_parameters::InspectContainerOptions>,
                )
                .await
                .ok()
                .and_then(|v| serde_json::to_value(v).ok())
                .unwrap_or(Value::Null);
            let labels: HashMap<String, String> =
                serde_json::from_value(value["Labels"].clone()).unwrap_or_default();
            let mut ports = Vec::new();
            let port_bindings = if inspect["State"]["Running"] == true {
                &inspect["NetworkSettings"]["Ports"]
            } else {
                &inspect["HostConfig"]["PortBindings"]
            };
            if let Some(bindings) = port_bindings.as_object() {
                for (target, entries) in bindings {
                    let Some((port, protocol)) = target.split_once('/') else {
                        continue;
                    };
                    for entry in entries.as_array().into_iter().flatten() {
                        if let (Ok(container_port), Ok(host_port)) =
                            (port.parse(), string(entry, "HostPort").parse())
                        {
                            ports.push(ContainerPort {
                                host_ip: if string(entry, "HostIp").is_empty() {
                                    "0.0.0.0".into()
                                } else {
                                    string(entry, "HostIp")
                                },
                                host_port,
                                container_port,
                                protocol: protocol.into(),
                            });
                        }
                    }
                }
            }
            let mounts = inspect["Mounts"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|m| MountSummary {
                    kind: string(m, "Type"),
                    source: string(m, "Source"),
                    destination: string(m, "Destination"),
                    read_only: !m["RW"].as_bool().unwrap_or(true),
                })
                .collect();
            result.containers.push(StandaloneContainer {
                provenance: labels.get("org.opencontainers.image.source").map(|source| {
                    Provenance {
                        kind: "source_metadata".into(),
                        source: source.clone(),
                        evidence: "org.opencontainers.image.source".into(),
                    }
                }),
                short_id: id.chars().take(12).collect(),
                id,
                name: value["Names"][0]
                    .as_str()
                    .unwrap_or("?")
                    .trim_start_matches('/')
                    .into(),
                image: string(&value, "Image"),
                image_id: string(&value, "ImageID"),
                state: string(&value, "State"),
                status: string(&value, "Status"),
                health: inspect["State"]["Health"]["Status"]
                    .as_str()
                    .map(str::to_owned),
                created: value["Created"].as_i64().unwrap_or_default(),
                labels,
                ports,
                mounts,
                networks: inspect["NetworkSettings"]["Networks"]
                    .as_object()
                    .map(|v| v.keys().cloned().collect())
                    .unwrap_or_default(),
                network_addresses: inspect["NetworkSettings"]["Networks"]
                    .as_object()
                    .map(|networks| {
                        networks
                            .iter()
                            .map(|(name, value)| {
                                (
                                    name.clone(),
                                    ["IPAddress", "GlobalIPv6Address"]
                                        .iter()
                                        .filter_map(|key| value[*key].as_str())
                                        .filter(|ip| !ip.is_empty())
                                        .map(str::to_owned)
                                        .collect(),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            });
        }
        for image in client
            .list_images(None::<bollard::query_parameters::ListImagesOptions>)
            .await?
        {
            let v = serde_json::to_value(image)?;
            result.images.push(ImageSummary {
                id: string(&v, "Id"),
                tags: serde_json::from_value(v["RepoTags"].clone()).unwrap_or_default(),
                size: v["Size"].as_i64().unwrap_or_default(),
                created: v["Created"].as_i64().unwrap_or_default(),
                containers: v["Containers"].as_i64().unwrap_or_default(),
                update_status: "unknown".into(),
            });
        }
        for network in client
            .list_networks(None::<bollard::query_parameters::ListNetworksOptions>)
            .await?
        {
            let v = serde_json::to_value(network)?;
            let id = string(&v, "Id");
            let inspect = client
                .inspect_network(
                    &id,
                    None::<bollard::query_parameters::InspectNetworkOptions>,
                )
                .await
                .ok()
                .and_then(|v| serde_json::to_value(v).ok())
                .unwrap_or(Value::Null);
            result.networks.push(NetworkSummary {
                id,
                name: string(&v, "Name"),
                driver: string(&v, "Driver"),
                scope: string(&v, "Scope"),
                internal: v["Internal"].as_bool().unwrap_or(false),
                containers: inspect["Containers"].as_object().map_or(0, |m| m.len()),
            });
        }
        let volumes = serde_json::to_value(
            client
                .list_volumes(None::<bollard::query_parameters::ListVolumesOptions>)
                .await?,
        )?;
        for v in volumes["Volumes"].as_array().into_iter().flatten() {
            result.volumes.push(VolumeSummary {
                name: string(v, "Name"),
                driver: string(v, "Driver"),
                mountpoint: string(v, "Mountpoint"),
                labels: serde_json::from_value(v["Labels"].clone()).unwrap_or_default(),
            });
        }
        Ok(result)
    }
    pub async fn inspect(&self, kind: &str, id: &str) -> anyhow::Result<Value> {
        let output = self.command().args([kind, "inspect", id]).output().await?;
        anyhow::ensure!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let values: Value = serde_json::from_slice(&output.stdout)?;
        Ok(values[0].clone())
    }
    pub async fn stats(&self, id: &str) -> anyhow::Result<Value> {
        let output = self
            .command()
            .args(["stats", "--no-stream", "--format", "{{json .}}", id])
            .output()
            .await?;
        anyhow::ensure!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(serde_json::from_slice(&output.stdout)?)
    }
    pub async fn run(&self, args: &[String]) -> anyhow::Result<String> {
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(900),
            self.command().args(args).output(),
        )
        .await??;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::ensure!(output.status.success(), "{stdout}\n{stderr}");
        Ok(format!("{stdout}\n{stderr}"))
    }
    pub async fn run_reported(
        &self,
        args: &[String],
        sender: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> anyhow::Result<String> {
        let mut child = self
            .command()
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        let mut stdout = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut stderr = BufReader::new(child.stderr.take().unwrap()).lines();
        let mut captured = std::collections::VecDeque::new();
        let drain = async {
            let mut out_open = true;
            let mut err_open = true;
            while out_open || err_open {
                let line = tokio::select! {
                    line = stdout.next_line(), if out_open => match line? { Some(line) => Some(line), None => { out_open=false; None } },
                    line = stderr.next_line(), if err_open => match line? { Some(line) => Some(line), None => { err_open=false; None } },
                };
                if let Some(line) = line {
                    let line: String = line.chars().take(4000).collect();
                    let _ = sender.send(line.clone());
                    captured.push_back(line);
                    if captured.len() > 500 {
                        captured.pop_front();
                    }
                }
            }
            Ok::<_, anyhow::Error>(())
        };
        tokio::time::timeout(std::time::Duration::from_secs(900), drain).await??;
        let status = child.wait().await?;
        let output = captured.into_iter().collect::<Vec<_>>().join("\n");
        anyhow::ensure!(status.success(), "{output}");
        Ok(output)
    }
    pub fn logs(&self, id: &str, tail: &str) -> anyhow::Result<tokio::process::Child> {
        Ok(self
            .command()
            .args(["logs", "--follow", "--timestamps", "--tail", tail, id])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?)
    }
    pub async fn watch_events(&self, sender: tokio::sync::mpsc::Sender<Value>) {
        loop {
            if let Ok(mut child) = self
                .command()
                .args(["events", "--format", "{{json .}}"])
                .stdout(std::process::Stdio::piped())
                .spawn()
            {
                if let Some(stdout) = child.stdout.take() {
                    let mut lines = BufReader::new(stdout).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        if let Ok(event) = serde_json::from_str::<Value>(&line) {
                            let event = json!({"action": event["Action"], "kind":event["Type"], "id": event["Actor"]["ID"], "name":event["Actor"]["Attributes"]["name"], "time":event["time"]});
                            if sender.send(event).await.is_err() {
                                return;
                            }
                        }
                    }
                }
                let _ = child.wait().await;
            }
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        }
    }
    pub async fn credential_diagnostics(&self) -> Vec<String> {
        let root = std::env::var("DOCKER_CONFIG")
            .unwrap_or_else(|_| format!("{}/.docker", std::env::var("HOME").unwrap_or_default()));
        let Ok(content) = tokio::fs::read_to_string(PathBuf::from(root).join("config.json")).await
        else {
            return Vec::new();
        };
        let Ok(config) = serde_json::from_str::<Value>(&content) else {
            return vec!["config.json Docker invalide".into()];
        };
        let mut helpers: Vec<&str> = config["credHelpers"]
            .as_object()
            .into_iter()
            .flat_map(|m| m.values())
            .filter_map(Value::as_str)
            .collect();
        if let Some(helper) = config["credsStore"].as_str() {
            helpers.push(helper);
        }
        helpers.into_iter().filter(|h| matches!(*h, "desktop" | "osxkeychain" | "wincred" | "desktop.exe")).map(|h| format!("Helper {h} incompatible dans ce conteneur Linux. Fournissez une configuration auths dédiée ou un helper Linux compatible.")).collect()
    }
    pub async fn image_update(&self, id: &str) -> anyhow::Result<Value> {
        let local = self.inspect("image", id).await?;
        let digests: Vec<String> = local["RepoDigests"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter_map(|s| s.split_once('@').map(|(_, d)| d.into()))
            .collect();
        if digests.is_empty() {
            return Ok(json!({"status":"local","checkedAt":chrono::Utc::now()}));
        }
        if local["RepoTags"]
            .as_array()
            .is_none_or(|tags| tags.is_empty())
        {
            return Ok(json!({"status":"digest_pinned","checkedAt":chrono::Utc::now()}));
        }
        let tag = local["RepoTags"]
            .as_array()
            .and_then(|v| v.first())
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("Image locale sans tag"))?;
        let output = self
            .command()
            .args([
                "buildx",
                "imagetools",
                "inspect",
                tag,
                "--format",
                "{{json .Manifest}}",
            ])
            .output()
            .await?;
        anyhow::ensure!(
            output.status.success(),
            "Vérification impossible : {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let remote: Value = serde_json::from_slice(&output.stdout)?;
        let text = remote.to_string();
        Ok(
            json!({ "status": if digests.is_empty() { "local" } else if digests.iter().any(|d| text.contains(d.as_str())) { "up_to_date" } else { "update_available" }, "checkedAt": chrono::Utc::now(), "tag": tag }),
        )
    }
}

pub fn string(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().into()
}
