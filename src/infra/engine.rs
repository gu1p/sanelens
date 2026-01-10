use std::env;
use std::thread;
use std::time::Duration;

use crate::domain::{EngineKind, Provider, Scope};
use crate::infra::compose::{
    collect_docker_container_ids, collect_podman_container_ids,
    collect_podman_container_ids_by_name, remove_project_pods, resolve_service_name_docker,
    resolve_service_name_podman,
};
use crate::infra::process::run_output;
use crate::support::args::has_flag;

#[derive(Clone)]
pub(crate) struct Engine {
    kind: EngineKind,
    connection: Option<String>,
    podman_cmd: Vec<String>,
    docker_cmd: Vec<String>,
}

impl Engine {
    pub(crate) fn new(kind: EngineKind, compose_cmd: &[String]) -> Self {
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

    pub(crate) fn with_connection(mut self, connection: Option<String>) -> Self {
        if let Some(conn) = connection {
            self.connection = Some(conn.clone());
            self.podman_cmd = vec!["podman".to_string(), "--connection".to_string(), conn];
        }
        self
    }

    pub(crate) fn connection(&self) -> Option<String> {
        self.connection.clone()
    }

    pub(crate) fn supports_watchdog(&self) -> bool {
        matches!(self.kind, EngineKind::Podman)
    }

    pub(crate) fn manual_log_follow(&self, subcommand: &str, detach_requested: bool) -> bool {
        matches!(self.kind, EngineKind::Podman) && subcommand == "up" && !detach_requested
    }

    pub(crate) fn follow_logs_in_thread(&self, subcommand: &str, detach_requested: bool) -> bool {
        matches!(self.kind, EngineKind::Docker) && subcommand == "up" && !detach_requested
    }

    pub(crate) fn emit_stdout_for_logs(&self, detach_requested: bool) -> bool {
        matches!(self.kind, EngineKind::Podman) && !detach_requested
    }

    pub(crate) fn collect_container_ids(&self, project_name: &str, scope: Scope) -> Vec<String> {
        match self.kind {
            EngineKind::Podman => {
                collect_podman_container_ids(&self.podman_cmd, project_name, scope)
            }
            EngineKind::Docker => {
                collect_docker_container_ids(&self.docker_cmd, project_name, scope)
            }
        }
    }

    pub(crate) fn resolve_service_name(&self, project_name: &str, cid: &str) -> String {
        match self.kind {
            EngineKind::Podman => resolve_service_name_podman(&self.podman_cmd, project_name, cid),
            EngineKind::Docker => resolve_service_name_docker(&self.docker_cmd, project_name, cid),
        }
    }

    pub(crate) fn logs_cmd(&self, cid: &str, timestamps_enabled: bool) -> Vec<String> {
        let mut cmd = match self.kind {
            EngineKind::Podman => self.podman_cmd.clone(),
            EngineKind::Docker => self.docker_cmd.clone(),
        };
        cmd.push("logs".to_string());
        cmd.push("--follow".to_string());
        if timestamps_enabled {
            cmd.push("--timestamps".to_string());
        }
        cmd.push(cid.to_string());
        cmd
    }

    pub(crate) fn cleanup_project(
        &self,
        compose_cmd: &[String],
        compose_file: &str,
        provider: Provider,
        project_name: &str,
        project_args: &[String],
    ) {
        if !matches!(self.kind, EngineKind::Podman) {
            return;
        }
        self.compose_down(compose_cmd, compose_file, provider, project_args);
        remove_project_pods(&self.podman_cmd, project_name);
        let mut ids = collect_podman_container_ids(&self.podman_cmd, project_name, Scope::All);
        ids.extend(collect_podman_container_ids_by_name(
            &self.podman_cmd,
            project_name,
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

    pub(crate) fn start_project_containers_with_retries(&self, project_name: &str) -> bool {
        if !matches!(self.kind, EngineKind::Podman) {
            return false;
        }
        let attempts = 3;
        for i in 0..attempts {
            if self.start_project_containers(project_name) {
                return true;
            }
            thread::sleep(Duration::from_secs(2 * (i + 1) as u64));
        }
        false
    }

    fn start_project_containers(&self, project_name: &str) -> bool {
        let ids = collect_podman_container_ids(&self.podman_cmd, project_name, Scope::All);
        if ids.is_empty() {
            return false;
        }
        let mut cmd = self.podman_cmd.clone();
        cmd.push("start".to_string());
        cmd.extend(ids);
        run_output(&cmd)
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    fn compose_down(
        &self,
        compose_cmd: &[String],
        compose_file: &str,
        provider: Provider,
        project_args: &[String],
    ) {
        let mut cmd = compose_cmd.to_vec();
        if provider == Provider::PodmanCompose && !has_flag(&cmd, &["--in-pod"]) {
            cmd.push("--in-pod".to_string());
            cmd.push("false".to_string());
        }
        cmd.push("-f".to_string());
        cmd.push(compose_file.to_string());
        cmd.extend(project_args.iter().cloned());
        cmd.push("down".to_string());
        cmd.push("--remove-orphans".to_string());
        let _ = run_output(&cmd);
    }
}

fn extract_connection(compose_cmd: &[String]) -> Option<String> {
    if compose_cmd.is_empty() || compose_cmd[0] != "podman" {
        return None;
    }
    for idx in 0..compose_cmd.len() {
        let arg = &compose_cmd[idx];
        if arg == "--connection" && idx + 1 < compose_cmd.len() {
            return Some(compose_cmd[idx + 1].clone());
        }
        if let Some(rest) = arg.strip_prefix("--connection=") {
            return Some(rest.to_string());
        }
    }
    None
}
