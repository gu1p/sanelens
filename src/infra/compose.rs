use std::collections::HashSet;
use std::env;

use crate::domain::{EngineKind, Scope};
use crate::infra::process::{command_exists, run_output, run_status};

pub struct ComposeSelection {
    pub compose_cmd: Vec<String>,
    pub engine: EngineKind,
}

pub fn detect_compose_cmd(
    preferred_engine: Option<EngineKind>,
) -> Result<ComposeSelection, String> {
    if let Some(selection) = selection_from_env(preferred_engine)? {
        return Ok(selection);
    }

    match preferred_engine {
        Some(EngineKind::Podman) => detect_podman_compose_cmd()
            .map(|cmd| ComposeSelection {
                compose_cmd: cmd,
                engine: EngineKind::Podman,
            })
            .ok_or_else(|| "Podman compose tool not found in PATH.".to_string()),
        Some(EngineKind::Docker) => detect_docker_compose_cmd()
            .map(|cmd| ComposeSelection {
                compose_cmd: cmd,
                engine: EngineKind::Docker,
            })
            .ok_or_else(|| "Docker compose tool not found in PATH.".to_string()),
        None => {
            if let Some(cmd) = detect_podman_compose_cmd() {
                return Ok(ComposeSelection {
                    compose_cmd: cmd,
                    engine: EngineKind::Podman,
                });
            }
            if let Some(cmd) = detect_docker_compose_cmd() {
                return Ok(ComposeSelection {
                    compose_cmd: cmd,
                    engine: EngineKind::Docker,
                });
            }
            Err("No compose tool found in PATH.".to_string())
        }
    }
}

fn selection_from_env(
    preferred_engine: Option<EngineKind>,
) -> Result<Option<ComposeSelection>, String> {
    let Ok(env_cmd) = env::var("COMPOSE_CMD") else {
        return Ok(None);
    };
    match shell_words::split(&env_cmd) {
        Ok(cmd) if !cmd.is_empty() => {
            if is_legacy_compose_cmd(&cmd) {
                return Err(
                    "COMPOSE_CMD must use `podman compose` or `docker compose`.".to_string()
                );
            }
            let inferred = infer_engine_kind(&cmd);
            if let Some(preferred) = preferred_engine {
                if inferred != preferred {
                    let engine_name = display_engine(preferred);
                    return Err(format!(
                        "COMPOSE_CMD does not match --engine {engine_name}."
                    ));
                }
            }
            Ok(Some(ComposeSelection {
                compose_cmd: cmd,
                engine: preferred_engine.unwrap_or(inferred),
            }))
        }
        _ => Err("COMPOSE_CMD is set but empty or invalid.".to_string()),
    }
}

fn infer_engine_kind(compose_cmd: &[String]) -> EngineKind {
    if let Some(cmd) = compose_cmd.first() {
        if cmd.contains("podman") {
            return EngineKind::Podman;
        }
    }
    EngineKind::Docker
}

fn is_legacy_compose_cmd(cmd: &[String]) -> bool {
    cmd.first()
        .is_some_and(|value| value.contains("podman-compose") || value.contains("docker-compose"))
}

const fn display_engine(kind: EngineKind) -> &'static str {
    match kind {
        EngineKind::Podman => "podman",
        EngineKind::Docker => "docker",
    }
}

fn detect_podman_compose_cmd() -> Option<Vec<String>> {
    if !command_exists("podman") {
        return None;
    }
    let mut probe = vec!["podman".to_string()];
    if let Ok(conn) = env::var("PODMAN_CONNECTION") {
        probe.push("--connection".to_string());
        probe.push(conn);
    }
    probe.push("compose".to_string());
    probe.push("version".to_string());
    let output = run_output(&probe).ok()?;
    if !output.status.success() {
        return None;
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if let Some(provider) = extract_external_compose_provider(&stderr) {
        if command_exists(&provider) {
            return Some(vec![provider]);
        }
    }
    let mut cmd = vec!["podman".to_string()];
    if let Ok(conn) = env::var("PODMAN_CONNECTION") {
        cmd.push("--connection".to_string());
        cmd.push(conn);
    }
    cmd.push("compose".to_string());
    Some(cmd)
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
    None
}

fn extract_external_compose_provider(stderr: &str) -> Option<String> {
    let marker = "Executing external compose provider \"";
    let start = stderr.find(marker)? + marker.len();
    let rest = &stderr[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

pub fn collect_podman_container_ids(
    podman_cmd: &[String],
    project_name: &str,
    scope: Scope,
) -> Vec<String> {
    let mut ids = HashSet::new();
    let base = build_podman_ps_cmd(podman_cmd, scope);
    let labels = [
        format!("label=io.podman.compose.project={project_name}"),
        format!("label=com.docker.compose.project={project_name}"),
    ];
    for label in &labels {
        let mut cmd = base.clone();
        cmd.push("--filter".to_string());
        cmd.push(label.to_string());
        cmd.push("-q".to_string());
        if let Ok(output) = run_output(&cmd) {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
                ids.insert(line.trim().to_string());
            }
        }
    }
    let mut list: Vec<String> = ids.into_iter().collect();
    list.sort();
    list
}

pub fn collect_podman_container_ids_by_label(
    podman_cmd: &[String],
    label_key: &str,
    label_value: &str,
    scope: Scope,
) -> Vec<String> {
    let mut cmd = build_podman_ps_cmd(podman_cmd, scope);
    cmd.push("--filter".to_string());
    cmd.push(format!("label={label_key}={label_value}"));
    cmd.push("-q".to_string());
    let mut ids = Vec::new();
    if let Ok(output) = run_output(&cmd) {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
            ids.push(line.trim().to_string());
        }
    }
    ids.sort();
    ids.dedup();
    ids
}

pub fn collect_podman_container_ids_by_labels(
    podman_cmd: &[String],
    labels: &[(&str, &str)],
    scope: Scope,
) -> Vec<String> {
    let mut cmd = build_podman_ps_cmd(podman_cmd, scope);
    for (key, value) in labels {
        cmd.push("--filter".to_string());
        cmd.push(format!("label={key}={value}"));
    }
    cmd.push("-q".to_string());
    let mut ids = Vec::new();
    if let Ok(output) = run_output(&cmd) {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
            ids.push(line.trim().to_string());
        }
    }
    ids.sort();
    ids.dedup();
    ids
}

pub fn collect_podman_container_ids_by_label_key(
    podman_cmd: &[String],
    label_key: &str,
    scope: Scope,
) -> Vec<String> {
    let mut cmd = build_podman_ps_cmd(podman_cmd, scope);
    cmd.push("--filter".to_string());
    cmd.push(format!("label={label_key}"));
    cmd.push("-q".to_string());
    let mut ids = Vec::new();
    if let Ok(output) = run_output(&cmd) {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
            ids.push(line.trim().to_string());
        }
    }
    ids.sort();
    ids.dedup();
    ids
}

fn build_podman_ps_cmd(podman_cmd: &[String], scope: Scope) -> Vec<String> {
    let mut cmd = podman_cmd.to_vec();
    cmd.push("ps".to_string());
    if matches!(scope, Scope::All) {
        cmd.push("-a".to_string());
    }
    cmd
}

pub fn collect_docker_container_ids_by_label(
    docker_cmd: &[String],
    label_key: &str,
    label_value: &str,
    scope: Scope,
) -> Vec<String> {
    let mut cmd = docker_cmd.to_vec();
    cmd.push("ps".to_string());
    if matches!(scope, Scope::All) {
        cmd.push("-a".to_string());
    }
    cmd.push("--filter".to_string());
    cmd.push(format!("label={label_key}={label_value}"));
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

pub fn collect_docker_container_ids_by_labels(
    docker_cmd: &[String],
    labels: &[(&str, &str)],
    scope: Scope,
) -> Vec<String> {
    let mut cmd = docker_cmd.to_vec();
    cmd.push("ps".to_string());
    if matches!(scope, Scope::All) {
        cmd.push("-a".to_string());
    }
    for (key, value) in labels {
        cmd.push("--filter".to_string());
        cmd.push(format!("label={key}={value}"));
    }
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

pub fn collect_docker_container_ids_by_label_key(
    docker_cmd: &[String],
    label_key: &str,
    scope: Scope,
) -> Vec<String> {
    let mut cmd = docker_cmd.to_vec();
    cmd.push("ps".to_string());
    if matches!(scope, Scope::All) {
        cmd.push("-a".to_string());
    }
    cmd.push("--filter".to_string());
    cmd.push(format!("label={label_key}"));
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

pub fn collect_podman_container_ids_by_name(
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
            if (name.starts_with(&format!("{project_name}-"))
                || name.starts_with(&format!("{project_name}_")))
                && !id.trim().is_empty()
            {
                ids.insert(id.trim().to_string());
            }
        }
    }
    ids.into_iter().collect()
}

pub fn remove_project_pods(podman_cmd: &[String], project_name: &str) {
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
            if (name == format!("pod_{project_name}")
                || name.starts_with(&format!("{project_name}-")))
                && !id.trim().is_empty()
            {
                pod_ids.push(id.trim().to_string());
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

pub fn resolve_service_name_podman(podman_cmd: &[String], project_name: &str, cid: &str) -> String {
    let label_keys = ["io.podman.compose.service", "com.docker.compose.service"];
    for label in &label_keys {
        let mut command = podman_cmd.to_vec();
        command.push("inspect".to_string());
        command.push("--format".to_string());
        command.push(format!("{{{{ index .Config.Labels \"{label}\" }}}}"));
        command.push(cid.to_string());
        if let Ok(output) = run_output(&command) {
            let candidate = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !candidate.is_empty() && candidate != "<no value>" {
                return candidate;
            }
        }
    }
    let mut command = podman_cmd.to_vec();
    command.push("inspect".to_string());
    command.push("--format".to_string());
    command.push("{{ .Name }}".to_string());
    command.push(cid.to_string());
    if let Ok(output) = run_output(&command) {
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

pub fn resolve_service_name_docker(docker_cmd: &[String], project_name: &str, cid: &str) -> String {
    let label_keys = ["com.docker.compose.service"];
    for label in &label_keys {
        let mut command = docker_cmd.to_vec();
        command.push("inspect".to_string());
        command.push("--format".to_string());
        command.push(format!("{{{{ index .Config.Labels \"{label}\" }}}}"));
        command.push(cid.to_string());
        if let Ok(output) = run_output(&command) {
            let candidate = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !candidate.is_empty() && candidate != "<no value>" {
                return candidate;
            }
        }
    }
    let mut command = docker_cmd.to_vec();
    command.push("inspect".to_string());
    command.push("--format".to_string());
    command.push("{{ .Name }}".to_string());
    command.push(cid.to_string());
    if let Ok(output) = run_output(&command) {
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
    let prefix = format!("{project_name}_");
    if result.starts_with(&prefix) {
        result = result[prefix.len()..].to_string();
    }
    let prefix = format!("{project_name}-");
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
