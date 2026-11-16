use crate::{
    AppState, api,
    config::Config,
    db::Database,
    docker::{DockerService, RuntimeSnapshot},
    jobs::JobManager,
    models::*,
    ports,
    scanner::ComposeScanner,
};
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tower::ServiceExt;

async fn state(root: PathBuf) -> Arc<AppState> {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let config = Config {
        listen: "127.0.0.1:8080".parse().unwrap(),
        compose_root: root.clone(),
        data_dir: root.join("data"),
        database_url: "sqlite::memory:".into(),
        docker_host: "unix:///nonexistent-compmaster-test.sock".into(),
        docker_cli: "/usr/bin/false".into(),
        static_dir: root.join("dist"),
        public_origin: "http://127.0.0.1:8080".into(),
        scan_concurrency: 2,
        max_job_history: 20,
    };
    Arc::new(AppState {
        scanner: ComposeScanner::new(root, config.docker_cli.clone(), 2),
        docker: DockerService::connect(&config.docker_host, config.docker_cli.clone()),
        jobs: JobManager::new(db.clone(), 20),
        config,
        db,
        inventory: Default::default(),
        csrf_token: "test-token".into(),
        file_lock: Default::default(),
        refresh_lock: Default::default(),
        docker_events: Default::default(),
    })
}

#[tokio::test]
async fn rejects_dns_rebinding_cross_origin_and_missing_csrf() {
    let temp = tempfile::tempdir().unwrap();
    let state = state(temp.path().to_owned()).await;
    let router = api::router(state);
    for (host, origin, token, expected) in [
        ("evil.test", None, None, StatusCode::FORBIDDEN),
        (
            "127.0.0.1:8080",
            Some("http://evil.test"),
            Some("test-token"),
            StatusCode::FORBIDDEN,
        ),
        ("127.0.0.1:8080", None, None, StatusCode::FORBIDDEN),
        (
            "127.0.0.1:8080",
            Some("http://127.0.0.1:8080"),
            Some("test-token"),
            StatusCode::OK,
        ),
    ] {
        let mut request = Request::builder()
            .method("PUT")
            .uri("/api/v1/settings")
            .header("Host", host)
            .header("Content-Type", "application/json");
        if let Some(origin) = origin {
            request = request.header("Origin", origin);
        }
        if let Some(token) = token {
            request = request.header("X-CSRF-Token", token);
        }
        let response = router
            .clone()
            .oneshot(request.body(Body::from("{\"theme\":\"dark\"}")).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        if response.status() == StatusCode::OK {
            assert!(response.headers().contains_key("content-security-policy"));
        }
    }
}

#[tokio::test]
async fn session_exposes_no_docker_credentials() {
    let temp = tempfile::tempdir().unwrap();
    let router = api::router(state(temp.path().to_owned()).await);
    let response = router
        .oneshot(
            Request::builder()
                .uri("/api/v1/session")
                .header("Host", "127.0.0.1:8080")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let value: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
    assert_eq!(value["csrfToken"], "test-token");
    assert!(value.get("auths").is_none());
}

#[tokio::test]
async fn persistence_survives_reconnection_and_cancels_interrupted_jobs() {
    let temp = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        temp.path().join("state.db").display()
    );
    let db = Database::connect(&url).await.unwrap();
    db.set("favorite:p", &true.into()).await.unwrap();
    let job = Job {
        id: "j".into(),
        target: "p".into(),
        action: "up".into(),
        state: JobState::Running,
        progress: 10,
        output: vec![],
        error: None,
        created_at: chrono::Utc::now(),
        finished_at: None,
    };
    db.save_job(&job, 20).await.unwrap();
    let reopened = Database::connect(&url).await.unwrap();
    assert_eq!(reopened.settings().await.unwrap()["favorite:p"], true);
    assert_eq!(reopened.jobs().await.unwrap()[0].state, JobState::Cancelled);
}

#[tokio::test]
async fn compose_labels_group_services_but_do_not_invent_files() {
    let temp = tempfile::tempdir().unwrap();
    let state = state(temp.path().to_owned()).await;
    let containers = ["web", "db"]
        .iter()
        .map(|service| StandaloneContainer {
            id: service.to_string(),
            name: format!("example-{service}-1"),
            state: "running".into(),
            labels: HashMap::from([
                ("com.docker.compose.project".into(), "example".into()),
                ("com.docker.compose.service".into(), service.to_string()),
            ]),
            ..Default::default()
        })
        .chain([StandaloneContainer {
            id: "standalone".into(),
            ..Default::default()
        }])
        .collect();
    let inventory = Inventory::merge(
        vec![],
        RuntimeSnapshot {
            connected: true,
            containers,
            ..Default::default()
        },
        &state.config,
    )
    .await;
    assert_eq!(inventory.projects.len(), 1);
    assert_eq!(inventory.projects[0].containers.len(), 2);
    assert_eq!(inventory.projects[0].state, ResourceState::RuntimeOnly);
    assert!(inventory.projects[0].definition.is_none());
    assert_eq!(inventory.standalone.len(), 1);
}

fn binding(owner: &str, service: &str, port: u16, protocol: &str) -> PortBinding {
    PortBinding {
        id: format!("{owner}:{service}"),
        host_ip: "0.0.0.0".into(),
        host_port: port,
        host_port_end: None,
        container_port: 80,
        protocol: protocol.into(),
        owner_id: owner.into(),
        owner_name: owner.into(),
        service: Some(service.into()),
        source: PortSource::Desired,
        status: PortStatus::Desired,
        conflict: None,
    }
}
#[test]
fn internal_conflicts_are_found_and_protocols_are_distinct() {
    let mut bindings = vec![
        binding("a", "web", 8000, "tcp"),
        binding("a", "api", 8000, "tcp"),
        binding("b", "dns", 8000, "udp"),
    ];
    ports::mark_conflicts(&mut bindings);
    assert!(bindings[0].conflict.is_some());
    assert!(bindings[1].conflict.is_some());
    assert!(bindings[2].conflict.is_none());
    assert_eq!(
        bindings[0].conflict.as_ref().unwrap().suggested_port,
        Some(8001)
    );
}

#[tokio::test]
async fn dynamic_port_range_requires_only_one_available_slot() {
    let mut range = binding("a", "web", 8000, "udp");
    range.host_port_end = Some(8001);
    let mut occupied = binding("b", "dns", 8000, "udp");
    occupied.status = PortStatus::DockerOccupied;
    let mut bindings = vec![range, occupied];
    assert!(ports::preflight(&bindings, "a", "127.0.0.1").await.is_ok());
    let mut occupied = binding("c", "dns2", 8001, "udp");
    occupied.status = PortStatus::DockerOccupied;
    bindings.push(occupied);
    assert!(ports::preflight(&bindings, "a", "127.0.0.1").await.is_err());
}

#[test]
fn canonical_port_ranges_are_retained_and_ephemeral_ports_are_skipped() {
    let ports = crate::scanner::canonical_ports(
        &serde_json::json!({"services":{"web":{"ports":[{"target":80,"published":"8000-8003","protocol":"tcp"},{"target":81,"published":"0"}]}}}),
    );
    assert_eq!(ports.len(), 1);
    assert_eq!(ports[0].host_port_end, Some(8003));
}

#[test]
fn identical_explicit_project_names_never_authorize_compose_mutations() {
    let project = |id: &str| ComposeDeployment {
        id: id.into(),
        name: "example".into(),
        state: ResourceState::Discovered,
        definition: None,
        containers: vec![],
        services: vec![],
        favorite: false,
        diagnostics: vec![],
    };
    let mut inventory = Inventory::default();
    inventory.projects = vec![project("a"), project("b")];
    assert!(crate::jobs::ensure_unique_project_name(&inventory, "a", "example").is_err());
    inventory.projects.pop();
    assert!(crate::jobs::ensure_unique_project_name(&inventory, "a", "example").is_ok());
}

#[cfg(unix)]
#[tokio::test]
async fn recursive_scan_honors_ignore_and_never_follows_symlinks() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("nested")).unwrap();
    std::fs::create_dir_all(root.path().join("ignored")).unwrap();
    let content = "services:\n  api:\n    image: alpine\n";
    for relative in [
        "first.yml",
        "second.yaml",
        "nested/custom.yaml",
        "ignored/no.yaml",
    ] {
        std::fs::write(root.path().join(relative), content).unwrap();
    }
    std::fs::write(root.path().join("not-compose.yaml"), "hello: world\n").unwrap();
    std::fs::write(root.path().join(".compmasterignore"), "ignored/\n").unwrap();
    std::fs::write(outside.path().join("secret.yaml"), content).unwrap();
    symlink(outside.path(), root.path().join("link")).unwrap();
    let cli = root.path().join("docker-stub");
    std::fs::write(
        &cli,
        "#!/bin/sh\nprintf '%s' '{\"services\":{\"api\":{\"image\":\"alpine\"}}}'\n",
    )
    .unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    let scanner = ComposeScanner::new(root.path().to_owned(), cli, 2);
    let definitions = scanner.scan().await;
    assert_eq!(definitions.len(), 3);
    assert!(definitions.iter().all(|d| d.valid));
    assert_ne!(definitions[0].project_name, definitions[1].project_name);
}

#[tokio::test]
async fn unresolved_compose_validation_never_authorizes_actions() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("stack.yml"),
        "services:\n  web:\n    image: nginx\n",
    )
    .unwrap();
    let definitions = ComposeScanner::new(temp.path().to_owned(), "/nonexistent-docker".into(), 1)
        .scan()
        .await;
    assert_eq!(definitions.len(), 1);
    assert!(!definitions[0].valid);
}

#[tokio::test]
#[ignore = "requires explicit opt-in and a local Docker daemon"]
async fn docker_lifecycle_network_volume_and_compose_validation() {
    assert_eq!(std::env::var("COMPMASTER_DOCKER_TESTS").as_deref(), Ok("1"));
    let host = std::env::var("DOCKER_HOST").expect("set DOCKER_HOST explicitly");
    let cli = PathBuf::from(std::env::var("DOCKER_CLI").unwrap_or_else(|_| "docker".into()));
    let service = DockerService::connect(&host, cli);
    let name = format!(
        "cm-test-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..12]
    );
    // A Drop guard removes only exact resources created by this test, even after an assertion fails.
    struct Cleanup {
        cli: PathBuf,
        host: String,
        name: String,
    }
    impl Drop for Cleanup {
        fn drop(&mut self) {
            for args in [
                vec!["container", "rm", "-f", &self.name],
                vec!["network", "rm", &self.name],
                vec!["volume", "rm", &self.name],
            ] {
                let _ = std::process::Command::new(&self.cli)
                    .env("DOCKER_HOST", &self.host)
                    .args(args)
                    .output();
            }
        }
    }
    let _cleanup = Cleanup {
        cli: service.cli.clone(),
        host,
        name: name.clone(),
    };
    service
        .run(&["network".into(), "create".into(), name.clone()])
        .await
        .unwrap();
    service
        .run(&["volume".into(), "create".into(), name.clone()])
        .await
        .unwrap();
    service
        .run(&[
            "run".into(),
            "-d".into(),
            "--name".into(),
            name.clone(),
            "--network".into(),
            name.clone(),
            "--mount".into(),
            format!("type=volume,src={name},dst=/test"),
            "alpine:latest".into(),
            "sleep".into(),
            "120".into(),
        ])
        .await
        .unwrap();
    let inspect = service.inspect("container", &name).await.unwrap();
    assert_eq!(inspect["State"]["Running"], true);
    let snapshot = service.snapshot().await;
    assert!(snapshot.connected);
    assert!(snapshot.containers.iter().any(|c| c.name == name));
    service.run(&["stop".into(), name.clone()]).await.unwrap();
    service.run(&["start".into(), name.clone()]).await.unwrap();
    assert_eq!(
        service.inspect("container", &name).await.unwrap()["State"]["Running"],
        true
    );
}
