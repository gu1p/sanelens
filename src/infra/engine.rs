use std::env;
use std::net::IpAddr;

use crate::domain::{EngineKind, Scope};
use crate::infra::compose::{
    collect_docker_container_ids, collect_podman_container_ids,
    collect_podman_container_ids_by_name, remove_project_pods, resolve_service_name_docker,
    resolve_service_name_podman,
};
use crate::infra::process::run_output;

pub struct ContainerInfo {
    pub id: String,
    pub service: Option<String>,
    pub ips: Vec<IpAddr>,
}

#[derive(Clone)]
pub struct Engine {
    kind: EngineKind,
    connection: Option<String>,
    podman_cmd: Vec<String>,
    docker_cmd: Vec<String>,
}

pub struct CleanupContext<'a> {
    pub compose_cmd: &'a [String],
    pub compose_file: &'a str,
    pub project_name: &'a str,
    pub project_args: &'a [String],
}

impl Engine {
    pub fn new(kind: EngineKind, compose_cmd: &[String]) -> Self {
        let connection = if matches!(kind, EngineKind::Podman) {
            env::var("PODMAN_CONNECTION")
                .ok()
                .or_else(|| extract_connection(compose_cmd))
        } else {
            None
        };
        let mut podman_cmd = vec!["podman".to_string()];
        if let Some(ref conn) = connection {
            podman_cmd.push("--connection".to_string());
            podman_cmd.push(conn.to_string());
        }
        let docker_cmd = vec!["docker".to_string()];
        Self {
            kind,
            connection,
            podman_cmd,
            docker_cmd,
        }
    }

    pub fn with_connection(mut self, connection: Option<String>) -> Self {
        if let Some(conn) = connection {
            self.connection = Some(conn.clone());
            self.podman_cmd = vec!["podman".to_string(), "--connection".to_string(), conn];
        }
        self
    }

    pub fn connection(&self) -> Option<String> {
        self.connection.clone()
    }

    pub const fn supports_watchdog(&self) -> bool {
        matches!(self.kind, EngineKind::Podman)
    }

    pub fn manual_log_follow(&self, subcommand: &str, detach_requested: bool) -> bool {
        matches!(self.kind, EngineKind::Podman) && subcommand == "up" && !detach_requested
    }

    pub fn follow_logs_in_thread(&self, subcommand: &str, detach_requested: bool) -> bool {
        matches!(self.kind, EngineKind::Docker) && subcommand == "up" && !detach_requested
    }

    pub const fn emit_stdout_for_logs(&self, detach_requested: bool) -> bool {
        matches!(self.kind, EngineKind::Podman) && !detach_requested
    }

    pub const fn is_podman(&self) -> bool {
        matches!(self.kind, EngineKind::Podman)
    }

    pub fn collect_container_ids(&self, project_name: &str, scope: Scope) -> Vec<String> {
        match self.kind {
            EngineKind::Podman => {
                collect_podman_container_ids(&self.podman_cmd, project_name, scope)
            }
            EngineKind::Docker => {
                collect_docker_container_ids(&self.docker_cmd, project_name, scope)
            }
        }
    }

    pub fn resolve_service_name(&self, project_name: &str, cid: &str) -> String {
        match self.kind {
            EngineKind::Podman => resolve_service_name_podman(&self.podman_cmd, project_name, cid),
            EngineKind::Docker => resolve_service_name_docker(&self.docker_cmd, project_name, cid),
        }
    }

    pub fn logs_cmd(&self, cid: &str, timestamps_enabled: bool) -> Vec<String> {
        let mut command = match self.kind {
            EngineKind::Podman => self.podman_cmd.clone(),
            EngineKind::Docker => self.docker_cmd.clone(),
        };
        command.push("logs".to_string());
        command.push("--follow".to_string());
        if timestamps_enabled {
            command.push("--timestamps".to_string());
        }
        command.push(cid.to_string());
        command
    }

    pub fn cleanup_project(&self, context: &CleanupContext<'_>) {
        if !matches!(self.kind, EngineKind::Podman) {
            return;
        }
        Self::compose_down(
            context.compose_cmd,
            context.compose_file,
            context.project_args,
        );
        remove_project_pods(&self.podman_cmd, context.project_name);
        let mut ids =
            collect_podman_container_ids(&self.podman_cmd, context.project_name, Scope::All);
        ids.extend(collect_podman_container_ids_by_name(
            &self.podman_cmd,
            context.project_name,
        ));
        ids.sort();
        ids.dedup();
        if !ids.is_empty() {
            let mut cmd = self.podman_cmd.clone();
            cmd.push("rm".to_string());
            cmd.push("-f".to_string());
            cmd.extend(ids);
            let _ = run_output(&cmd);
        }
    }

    fn compose_down(compose_cmd: &[String], compose_file: &str, project_args: &[String]) {
        let mut cmd = compose_cmd.to_vec();
        cmd.push("-f".to_string());
        cmd.push(compose_file.to_string());
        cmd.extend(project_args.iter().cloned());
        cmd.push("down".to_string());
        cmd.push("--remove-orphans".to_string());
        let _ = run_output(&cmd);
    }

    pub fn inspect_containers(&self, ids: &[String]) -> Vec<ContainerInfo> {
        if ids.is_empty() {
            return Vec::new();
        }
        let mut cmd = match self.kind {
            EngineKind::Podman => self.podman_cmd.clone(),
            EngineKind::Docker => self.docker_cmd.clone(),
        };
        cmd.push("inspect".to_string());
        cmd.extend(ids.iter().cloned());
        let Ok(output) = run_output(&cmd) else {
            return Vec::new();
        };
        let value: serde_json::Value = match serde_json::from_slice(&output.stdout) {
            Ok(value) => value,
            Err(_) => return Vec::new(),
        };
        let Some(list) = value.as_array() else {
            return Vec::new();
        };
        let mut info = Vec::new();
        for item in list {
            let id = item
                .get("Id")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string();
            let labels = item
                .get("Config")
                .and_then(|value| value.get("Labels"))
                .and_then(|value| value.as_object());
            let service = labels
                .and_then(|map| {
                    map.get("com.docker.compose.service")
                        .or_else(|| map.get("io.podman.compose.service"))
                })
                .and_then(|value| value.as_str())
                .map(ToString::to_string);
            let ip_addresses = extract_ips(item);
            info.push(ContainerInfo {
                id,
                service,
                ips: ip_addresses,
            });
        }
        info
    }
}

fn extract_connection(compose_cmd: &[String]) -> Option<String> {
    if !matches!(compose_cmd.first(), Some(arg) if arg == "podman") {
        return None;
    }
    for (idx, arg) in compose_cmd.iter().enumerate() {
        if arg == "--connection" {
            if let Some(next) = compose_cmd.get(idx + 1) {
                return Some(next.clone());
            }
            continue;
        }
        if let Some(rest) = arg.strip_prefix("--connection=") {
            return Some(rest.to_string());
        }
    }
    None
}

fn extract_ips(container: &serde_json::Value) -> Vec<IpAddr> {
    let mut ips = Vec::new();
    let Some(networks) = container
        .get("NetworkSettings")
        .and_then(|value| value.get("Networks"))
        .and_then(|value| value.as_object())
    else {
        return ips;
    };
    for network in networks.values() {
        let parsed = ["IPAddress", "IpAddress", "ip_address", "ipAddress"]
            .iter()
            .filter_map(|key| {
                network
                    .get(*key)
                    .and_then(|value| value.as_str())
                    .and_then(|ip| ip.parse::<IpAddr>().ok())
            });
        ips.extend(parsed);
    }
    ips
}
