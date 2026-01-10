use std::collections::HashSet;
use std::env;

use crate::domain::{EngineKind, Provider, Scope};
use crate::infra::process::{command_exists, run_output, run_status};

pub(crate) struct ComposeSelection {
    pub(crate) compose_cmd: Vec<String>,
    pub(crate) engine: EngineKind,
}

pub(crate) fn detect_compose_cmd(preferred_engine: Option<EngineKind>) -> ComposeSelection {
    if let Ok(env_cmd) = env::var("COMPOSE_CMD") {
        match shell_words::split(&env_cmd) {
            Ok(cmd) if !cmd.is_empty() => {
                let inferred = infer_engine_kind(&cmd);
                if let Some(preferred) = preferred_engine {
                    if inferred != preferred {
                        eprintln!(
                            "COMPOSE_CMD does not match --engine {}.",
                            display_engine(preferred)
                        );
                        std::process::exit(1);
                    }
                }
                return ComposeSelection {
                    compose_cmd: cmd,
                    engine: preferred_engine.unwrap_or(inferred),
                };
            }
            _ => {
                eprintln!("COMPOSE_CMD is set but empty or invalid.");
                std::process::exit(1);
            }
        }
    }

    match preferred_engine {
        Some(EngineKind::Podman) => {
            if let Some(cmd) = detect_podman_compose_cmd() {
                return ComposeSelection {
                    compose_cmd: cmd,
                    engine: EngineKind::Podman,
                };
            }
            eprintln!("Podman compose tool not found in PATH.");
            std::process::exit(1);
        }
        Some(EngineKind::Docker) => {
            if let Some(cmd) = detect_docker_compose_cmd() {
                return ComposeSelection {
                    compose_cmd: cmd,
                    engine: EngineKind::Docker,
                };
            }
            eprintln!("Docker compose tool not found in PATH.");
            std::process::exit(1);
        }
        None => {
            if let Some(cmd) = detect_podman_compose_cmd() {
                return ComposeSelection {
                    compose_cmd: cmd,
                    engine: EngineKind::Podman,
                };
            }
            if let Some(cmd) = detect_docker_compose_cmd() {
                return ComposeSelection {
                    compose_cmd: cmd,
                    engine: EngineKind::Docker,
                };
            }
            eprintln!("No compose tool found in PATH.");
            std::process::exit(1);
        }
    }
}

pub(crate) fn detect_provider(compose_cmd: &[String]) -> Provider {
    if compose_cmd.is_empty() {
        return Provider::Other;
    }
    if compose_cmd[0] == "podman-compose" {
        return Provider::PodmanCompose;
    }
    if compose_cmd.len() >= 2 && compose_cmd[0] == "podman" && compose_cmd[1] == "compose" {
        let mut cmd = compose_cmd.to_vec();
        cmd.push("version".to_string());
        if let Ok(output) = run_output(&cmd) {
            let mut combined = String::new();
            combined.push_str(&String::from_utf8_lossy(&output.stdout));
            combined.push_str(&String::from_utf8_lossy(&output.stderr));
            if combined.to_lowercase().contains("podman-compose") {
                return Provider::PodmanCompose;
            }
        }
    }
    Provider::Other
}

fn infer_engine_kind(compose_cmd: &[String]) -> EngineKind {
    if let Some(cmd) = compose_cmd.first() {
        if cmd.contains("podman") {
            return EngineKind::Podman;
        }
    }
    EngineKind::Docker
}

fn display_engine(kind: EngineKind) -> &'static str {
    match kind {
        EngineKind::Podman => "podman",
        EngineKind::Docker => "docker",
    }
}

fn detect_podman_compose_cmd() -> Option<Vec<String>> {
    if command_exists("podman") {
        if run_status(&[
            "podman".to_string(),
            "compose".to_string(),
            "version".to_string(),
        ]) {
            let mut cmd = vec!["podman".to_string()];
            if let Ok(conn) = env::var("PODMAN_CONNECTION") {
                cmd.push("--connection".to_string());
                cmd.push(conn);
            }
            cmd.push("compose".to_string());
            return Some(cmd);
        }
    }
    if command_exists("podman-compose") {
        return Some(vec!["podman-compose".to_string()]);
    }
    None
}

fn detect_docker_compose_cmd() -> Option<Vec<String>> {
    if command_exists("docker")
        && run_status(&[
            "docker".to_string(),
            "compose".to_string(),
            "version".to_string(),
        ])
    {
        return Some(vec!["docker".to_string(), "compose".to_string()]);
    }
    if command_exists("docker-compose") {
        return Some(vec!["docker-compose".to_string()]);
    }
    None
}

pub(crate) fn collect_podman_container_ids(
    podman_cmd: &[String],
    project_name: &str,
    scope: Scope,
) -> Vec<String> {
    let mut ids = HashSet::new();
    let base = build_podman_ps_cmd(podman_cmd, scope);
    let labels = [
        format!("label=io.podman.compose.project={}", project_name),
        format!("label=com.docker.compose.project={}", project_name),
    ];
    for label in &labels {
        let mut cmd = base.clone();
        cmd.push("--filter".to_string());
        cmd.push(label.to_string());
        cmd.push("-q".to_string());
        if let Ok(output) = run_output(&cmd) {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if !line.trim().is_empty() {
                    ids.insert(line.trim().to_string());
                }
            }
        }
    }
    let mut list: Vec<String> = ids.into_iter().collect();
    list.sort();
    list
}

fn build_podman_ps_cmd(podman_cmd: &[String], scope: Scope) -> Vec<String> {
    let mut cmd = podman_cmd.to_vec();
    cmd.push("ps".to_string());
    if matches!(scope, Scope::All) {
        cmd.push("-a".to_string());
    }
    cmd
}

pub(crate) fn collect_docker_container_ids(
    docker_cmd: &[String],
    project_name: &str,
    scope: Scope,
) -> Vec<String> {
    let mut cmd = docker_cmd.to_vec();
    cmd.push("ps".to_string());
    if matches!(scope, Scope::All) {
        cmd.push("-a".to_string());
    }
    cmd.push("--filter".to_string());
    cmd.push(format!("label=com.docker.compose.project={}", project_name));
    cmd.push("-q".to_string());
    let mut ids = Vec::new();
    if let Ok(output) = run_output(&cmd) {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if !line.trim().is_empty() {
                ids.push(line.trim().to_string());
            }
        }
    }
    ids.sort();
    ids.dedup();
    ids
}

pub(crate) fn collect_podman_container_ids_by_name(
    podman_cmd: &[String],
    project_name: &str,
) -> Vec<String> {
    let mut cmd = podman_cmd.to_vec();
    cmd.push("ps".to_string());
    cmd.push("-a".to_string());
    cmd.push("--format".to_string());
    cmd.push("{{.ID}} {{.Names}}".to_string());
    let mut ids = HashSet::new();
    if let Ok(output) = run_output(&cmd) {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let mut parts = line.splitn(2, ' ');
            let id = parts.next().unwrap_or("");
            let name = parts.next().unwrap_or("");
            if name.starts_with(&format!("{}-", project_name))
                || name.starts_with(&format!("{}_", project_name))
            {
                if !id.trim().is_empty() {
                    ids.insert(id.trim().to_string());
                }
            }
        }
    }
    ids.into_iter().collect()
}

pub(crate) fn remove_project_pods(podman_cmd: &[String], project_name: &str) {
    let mut cmd = podman_cmd.to_vec();
    cmd.push("pod".to_string());
    cmd.push("ps".to_string());
    cmd.push("-a".to_string());
    cmd.push("--format".to_string());
    cmd.push("{{.Id}} {{.Name}}".to_string());
    let mut pod_ids = Vec::new();
    if let Ok(output) = run_output(&cmd) {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let mut parts = line.splitn(2, ' ');
            let id = parts.next().unwrap_or("");
            let name = parts.next().unwrap_or("");
            if name == format!("pod_{}", project_name)
                || name.starts_with(&format!("{}-", project_name))
            {
                if !id.trim().is_empty() {
                    pod_ids.push(id.trim().to_string());
                }
            }
        }
    }
    if pod_ids.is_empty() {
        return;
    }
    let mut rm_cmd = podman_cmd.to_vec();
    rm_cmd.push("pod".to_string());
    rm_cmd.push("rm".to_string());
    rm_cmd.push("-f".to_string());
    rm_cmd.extend(pod_ids);
    let _ = run_output(&rm_cmd);
}

pub(crate) fn resolve_service_name_podman(
    podman_cmd: &[String],
    project_name: &str,
    cid: &str,
) -> String {
    let label_keys = ["io.podman.compose.service", "com.docker.compose.service"];
    for label in &label_keys {
        let mut cmd = podman_cmd.to_vec();
        cmd.push("inspect".to_string());
        cmd.push("--format".to_string());
        cmd.push(format!("{{{{ index .Config.Labels \"{}\" }}}}", label));
        cmd.push(cid.to_string());
        if let Ok(output) = run_output(&cmd) {
            let candidate = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !candidate.is_empty() && candidate != "<no value>" {
                return candidate;
            }
        }
    }
    let mut cmd = podman_cmd.to_vec();
    cmd.push("inspect".to_string());
    cmd.push("--format".to_string());
    cmd.push("{{ .Name }}".to_string());
    cmd.push(cid.to_string());
    if let Ok(output) = run_output(&cmd) {
        let mut name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if name.starts_with('/') {
            name.remove(0);
        }
        name = strip_service_suffix(&name, project_name);
        if !name.is_empty() {
            return name;
        }
    }
    cid.to_string()
}

pub(crate) fn resolve_service_name_docker(
    docker_cmd: &[String],
    project_name: &str,
    cid: &str,
) -> String {
    let label_keys = ["com.docker.compose.service"];
    for label in &label_keys {
        let mut cmd = docker_cmd.to_vec();
        cmd.push("inspect".to_string());
        cmd.push("--format".to_string());
        cmd.push(format!("{{{{ index .Config.Labels \"{}\" }}}}", label));
        cmd.push(cid.to_string());
        if let Ok(output) = run_output(&cmd) {
            let candidate = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !candidate.is_empty() && candidate != "<no value>" {
                return candidate;
            }
        }
    }
    let mut cmd = docker_cmd.to_vec();
    cmd.push("inspect".to_string());
    cmd.push("--format".to_string());
    cmd.push("{{ .Name }}".to_string());
    cmd.push(cid.to_string());
    if let Ok(output) = run_output(&cmd) {
        let mut name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if name.starts_with('/') {
            name.remove(0);
        }
        name = strip_service_suffix(&name, project_name);
        if !name.is_empty() {
            return name;
        }
    }
    cid.to_string()
}

fn strip_service_suffix(name: &str, project_name: &str) -> String {
    let mut result = name.to_string();
    let prefix = format!("{}_", project_name);
    if result.starts_with(&prefix) {
        result = result[prefix.len()..].to_string();
    }
    let prefix = format!("{}-", project_name);
    if result.starts_with(&prefix) {
        result = result[prefix.len()..].to_string();
    }
    if result.ends_with("_1") {
        let len = result.len();
        result.truncate(len - 2);
    }
    if result.ends_with("-1") {
        let len = result.len();
        result.truncate(len - 2);
    }
    result
}
