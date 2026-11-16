use crate::models::*;
use std::{net::IpAddr, time::Duration};

pub fn from_inventory<'a>(
    projects: impl Iterator<Item = &'a ComposeDeployment>,
    standalone: &[StandaloneContainer],
) -> Vec<PortBinding> {
    let mut result = Vec::new();
    for project in projects {
        for container in &project.containers {
            append_container(&mut result, container, &project.id, &project.name);
        }
        if let Some(definition) = &project.definition {
            for (index, port) in definition.desired_ports.iter().enumerate() {
                // Keep desired bindings visible only when not already represented by runtime state.
                if result.iter().any(|p| {
                    p.owner_id == project.id
                        && p.host_port >= port.host_port
                        && p.host_port <= port.host_port_end.unwrap_or(port.host_port)
                        && p.service.as_deref() == Some(&port.service)
                        && p.protocol == port.protocol
                        && p.host_ip == port.host_ip
                }) {
                    continue;
                }
                result.push(PortBinding {
                    id: format!("{}:desired:{index}", project.id),
                    host_ip: port.host_ip.clone(),
                    host_port: port.host_port,
                    host_port_end: port.host_port_end,
                    container_port: port.container_port,
                    protocol: port.protocol.clone(),
                    owner_id: project.id.clone(),
                    owner_name: project.name.clone(),
                    service: Some(port.service.clone()),
                    source: PortSource::Desired,
                    status: PortStatus::Desired,
                    conflict: None,
                });
            }
        }
    }
    for container in standalone {
        append_container(&mut result, container, &container.id, &container.name);
    }
    result.sort_by_key(|p| (p.host_port, p.protocol.clone()));
    result
}

fn append_container(
    result: &mut Vec<PortBinding>,
    container: &StandaloneContainer,
    owner: &str,
    name: &str,
) {
    for (index, port) in container.ports.iter().enumerate() {
        result.push(PortBinding {
            id: format!("{}:{index}", container.id),
            host_ip: port.host_ip.clone(),
            host_port: port.host_port,
            host_port_end: None,
            container_port: port.container_port,
            protocol: port.protocol.clone(),
            owner_id: owner.into(),
            owner_name: name.into(),
            service: container.labels.get("com.docker.compose.service").cloned(),
            source: PortSource::Runtime,
            status: if matches!(
                container.state.as_str(),
                "running" | "restarting" | "paused"
            ) {
                PortStatus::DockerOccupied
            } else {
                PortStatus::StoppedReservation
            },
            conflict: None,
        });
    }
}

pub fn ip_overlap(left: &str, right: &str) -> bool {
    let wildcard = |s: &str| matches!(s, "" | "0.0.0.0" | "::" | "[::]");
    wildcard(left)
        || wildcard(right)
        || left == right
        || left
            .parse::<IpAddr>()
            .ok()
            .zip(right.parse::<IpAddr>().ok())
            .is_some_and(|(a, b)| a == b)
}

pub fn mark_conflicts(bindings: &mut [PortBinding]) {
    for index in 0..bindings.len() {
        let other = bindings
            .iter()
            .enumerate()
            .find(|(j, other)| {
                *j != index
                    && (other.owner_id != bindings[index].owner_id
                        || other.service != bindings[index].service)
                    && port_overlap(other, &bindings[index])
                    && other.protocol == bindings[index].protocol
                    && ip_overlap(&other.host_ip, &bindings[index].host_ip)
            })
            .map(|(_, p)| (p.owner_name.clone(), p.status));
        if let Some((name, status)) = other {
            let occupied = matches!(
                status,
                PortStatus::DockerOccupied | PortStatus::ExternalDetected
            );
            bindings[index].conflict = Some(PortConflict {
                kind: if occupied { "occupied" } else { "potential" }.into(),
                message: format!("Port partagé avec {name}"),
                conflicting_owner: Some(name),
                suggested_port: suggestion(bindings, bindings[index].host_port),
            });
        }
    }
}

pub fn suggestion(bindings: &[PortBinding], port: u16) -> Option<u16> {
    (u32::from(port) + 1..=65535)
        .take(200)
        .map(|p| p as u16)
        .find(|p| {
            !bindings
                .iter()
                .any(|b| b.host_port <= *p && b.host_port_end.unwrap_or(b.host_port) >= *p)
        })
}

fn port_overlap(a: &PortBinding, b: &PortBinding) -> bool {
    a.host_port <= b.host_port_end.unwrap_or(b.host_port)
        && b.host_port <= a.host_port_end.unwrap_or(a.host_port)
}

pub async fn discover_external(bindings: &mut Vec<PortBinding>, probe_host: &str) {
    use futures_util::{StreamExt, stream};
    let active: std::collections::HashSet<_> = bindings
        .iter()
        .filter(|p| p.status == PortStatus::DockerOccupied && p.protocol == "tcp")
        .map(|p| p.host_port)
        .collect();
    let requested: std::collections::BTreeSet<_> = bindings
        .iter()
        .filter(|p| p.protocol == "tcp" && !active.contains(&p.host_port))
        .map(|p| p.host_port)
        .collect();
    let occupied: Vec<_> = stream::iter(requested.into_iter().take(100))
        .map(|port| async move { (port, probe(probe_host, port).await) })
        .buffer_unordered(8)
        .filter_map(|(port, status)| {
            std::future::ready((status == "external_detected").then_some(port))
        })
        .collect()
        .await;
    for port in occupied {
        bindings.push(PortBinding {
            id: format!("host:{port}:tcp"),
            host_ip: "0.0.0.0".into(),
            host_port: port,
            host_port_end: None,
            container_port: 0,
            protocol: "tcp".into(),
            owner_id: format!("host:{port}"),
            owner_name: "Service hôte (sondage indicatif)".into(),
            service: None,
            source: PortSource::External,
            status: PortStatus::ExternalDetected,
            conflict: None,
        });
    }
    for binding in bindings.iter_mut() {
        binding.conflict = None;
    }
    mark_conflicts(bindings);
}

pub async fn probe(host: &str, port: u16) -> &'static str {
    match tokio::time::timeout(
        Duration::from_millis(300),
        tokio::net::TcpStream::connect((host, port)),
    )
    .await
    {
        Ok(Ok(_)) => "external_detected",
        Ok(Err(error)) if error.kind() == std::io::ErrorKind::ConnectionRefused => "probably_free",
        _ => "unknown",
    }
}

pub async fn preflight(
    bindings: &[PortBinding],
    owner: &str,
    probe_host: &str,
) -> anyhow::Result<()> {
    for binding in bindings.iter().filter(|p| p.owner_id == owner) {
        if let Some(end) = binding.host_port_end {
            let mut all_occupied = true;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
            for port in binding.host_port..=end {
                if tokio::time::Instant::now() >= deadline {
                    all_occupied = false;
                    break;
                }
                let docker_busy = bindings.iter().any(|p| {
                    p.owner_id != owner
                        && p.host_port == port
                        && p.protocol == binding.protocol
                        && p.status == PortStatus::DockerOccupied
                        && ip_overlap(&p.host_ip, &binding.host_ip)
                });
                if !docker_busy
                    && (binding.protocol != "tcp"
                        || probe(probe_host, port).await != "external_detected")
                {
                    all_occupied = false;
                    break;
                }
            }
            anyhow::ensure!(
                !all_occupied,
                "Tous les ports de {} à {} sont occupés",
                binding.host_port,
                end
            );
            continue;
        }
        if let Some(other) = bindings.iter().find(|p| {
            p.owner_id == owner
                && p.service != binding.service
                && p.host_port_end.is_none()
                && p.host_port == binding.host_port
                && p.protocol == binding.protocol
                && ip_overlap(&p.host_ip, &binding.host_ip)
        }) {
            anyhow::bail!(
                "Conflit interne sur {}/{} entre {:?} et {:?}",
                binding.host_port,
                binding.protocol,
                binding.service,
                other.service
            );
        }
        if let Some(other) = bindings.iter().find(|p| {
            p.owner_id != owner
                && p.host_port == binding.host_port
                && p.protocol == binding.protocol
                && ip_overlap(&p.host_ip, &binding.host_ip)
                && p.status == PortStatus::DockerOccupied
        }) {
            anyhow::bail!(
                "Port {}/{} occupé par {}. Suggestion : {:?}",
                binding.host_port,
                binding.protocol,
                other.owner_name,
                suggestion(bindings, binding.host_port)
            );
        }
        let own_active = bindings.iter().any(|p| {
            p.owner_id == owner
                && p.host_port == binding.host_port
                && p.protocol == binding.protocol
                && p.status == PortStatus::DockerOccupied
        });
        if !own_active
            && binding.protocol == "tcp"
            && probe(probe_host, binding.host_port).await == "external_detected"
        {
            anyhow::bail!(
                "Port TCP {} : service externe détecté sur {}",
                binding.host_port,
                probe_host
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wildcard_conflicts_are_conservative() {
        assert!(ip_overlap("0.0.0.0", "127.0.0.1"));
        assert!(ip_overlap("::", "127.0.0.1"));
        assert!(!ip_overlap("127.0.0.1", "192.168.1.2"));
    }
    #[tokio::test]
    async fn probe_detects_listener() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        assert_eq!(
            probe("127.0.0.1", listener.local_addr().unwrap().port()).await,
            "external_detected"
        );
    }
}
