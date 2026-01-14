use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_yaml::{Mapping, Value};

use crate::support::args::extract_compose_global_args;
use crate::support::constants::{
    COMPOSE_FILE_LABEL, DERIVED_COMPOSE_LABEL, PROJECT_NAME_LABEL, RUN_ID_LABEL, SERVICE_LABEL,
    STARTED_AT_LABEL,
};

#[derive(Clone)]
pub struct DerivedCompose {
    pub path: PathBuf,
    pub run_dir: PathBuf,
    pub proxy_services: HashSet<String>,
    pub app_service_map: HashMap<String, String>,
    pub egress_proxy: Option<String>,
}

#[derive(Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct DeriveConfig {
    pub run_id: String,
    pub run_started_at: String,
    pub envoy_image: String,
    pub enable_traffic: bool,
    pub enable_egress: bool,
    pub compose_cmd: Vec<String>,
    pub compose_args: Vec<String>,
    pub compose_file_from_args: bool,
    pub disable_pods: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ProxyProtocol {
    Http,
    Tcp,
}

struct RunLabelContext<'a> {
    run_id: &'a str,
    compose_file: &'a str,
    derived_compose: &'a str,
    started_at: &'a str,
    project_name: &'a str,
}

#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
pub fn derive_compose(
    compose_file: &str,
    project_name: &str,
    config: &DeriveConfig,
) -> Result<DerivedCompose, String> {
    let compose_path = to_absolute_path(compose_file)
        .map_err(|err| format!("failed to resolve compose path: {err}"))?;
    let mut doc = load_compose_doc(&compose_path, project_name, config)?;
    set_compose_name(&mut doc, project_name);
    let compose_dir = compose_path.parent().unwrap_or_else(|| Path::new("."));
    let out_dir = compose_dir.join(".sanelens").join(project_name);
    fs::create_dir_all(&out_dir).map_err(|err| format!("failed to create derived dir: {err}"))?;
    let compose_file_label = compose_path.to_string_lossy().into_owned();
    let derived_path = out_dir.join("compose.derived.yaml");
    let derived_compose_label = derived_path.to_string_lossy().into_owned();
    let run_labels = RunLabelContext {
        run_id: &config.run_id,
        compose_file: &compose_file_label,
        derived_compose: &derived_compose_label,
        started_at: &config.run_started_at,
        project_name,
    };

    rewrite_top_level_paths(&mut doc, compose_dir);
    if config.disable_pods {
        disable_podman_pods(&mut doc);
    }

    let network_names = if config.enable_traffic {
        collect_network_names(&doc)
    } else {
        Vec::new()
    };
    let service_names = if config.enable_traffic {
        collect_service_names(&doc)?
    } else {
        Vec::new()
    };

    let Some(Value::Mapping(services)) = doc.get_mut("services") else {
        return Err("compose file missing services".to_string());
    };

    if !config.enable_traffic {
        for (name, service_value) in services.iter_mut() {
            let Some(service_name) = name.as_str() else {
                continue;
            };
            let Value::Mapping(service) = service_value else {
                continue;
            };
            rewrite_service_paths(service, compose_dir);
            add_run_labels(service, service_name, &run_labels);
        }
        let payload = serde_yaml::to_string(&doc)
            .map_err(|err| format!("serialize compose failed: {err}"))?;
        fs::write(&derived_path, payload)
            .map_err(|err| format!("write derived compose failed: {err}"))?;
        return Ok(DerivedCompose {
            path: derived_path,
            run_dir: out_dir,
            proxy_services: HashSet::new(),
            app_service_map: HashMap::new(),
            egress_proxy: None,
        });
    }

    let envoy_dir = out_dir.join("envoy");
    fs::create_dir_all(&envoy_dir).map_err(|err| format!("failed to create derived dir: {err}"))?;
    let tap_dir = out_dir.join("tap");
    fs::create_dir_all(&tap_dir).map_err(|err| format!("failed to create tap dir: {err}"))?;

    let mut new_services = Mapping::new();
    let mut proxy_services = HashSet::new();
    let mut app_service_map = HashMap::new();
    let mut proxy_app_map = HashMap::new();
    let mut no_proxy_hosts = Vec::new();
    for name in &service_names {
        no_proxy_hosts.push(name.clone());
    }
    no_proxy_hosts.push("localhost".to_string());
    no_proxy_hosts.push("127.0.0.1".to_string());
    let no_proxy_value = no_proxy_hosts.join(",");

    for name in service_names {
        let key = Value::String(name.clone());
        let service_value = services.get(&key).cloned().unwrap_or(Value::Null);
        let mut service = match service_value {
            Value::Mapping(map) => map,
            other => {
                new_services.insert(key, other);
                continue;
            }
        };
        rewrite_service_paths(&mut service, compose_dir);
        let network_mode = get_string(&service, "network_mode");
        if network_mode.as_deref() == Some("host") || network_mode.as_deref() == Some("none") {
            add_run_labels(&mut service, &name, &run_labels);
            new_services.insert(key, Value::Mapping(service));
            continue;
        }
        let ports = extract_ports(&service);
        if ports.is_empty() {
            if config.enable_egress {
                ensure_env_var(
                    &mut service,
                    "HTTP_PROXY",
                    "http://sanelens-egress-proxy:15001",
                );
                ensure_env_var(
                    &mut service,
                    "HTTPS_PROXY",
                    "http://sanelens-egress-proxy:15001",
                );
                merge_env_var(&mut service, "NO_PROXY", &no_proxy_value);
            }
            add_run_labels(&mut service, &name, &run_labels);
            new_services.insert(key, Value::Mapping(service));
            continue;
        }
        let protocol_override = read_proxy_protocol(&service);
        if protocol_override == Some("off".to_string()) {
            if config.enable_egress {
                ensure_env_var(
                    &mut service,
                    "HTTP_PROXY",
                    "http://sanelens-egress-proxy:15001",
                );
                ensure_env_var(
                    &mut service,
                    "HTTPS_PROXY",
                    "http://sanelens-egress-proxy:15001",
                );
                merge_env_var(&mut service, "NO_PROXY", &no_proxy_value);
            }
            add_run_labels(&mut service, &name, &run_labels);
            new_services.insert(key, Value::Mapping(service));
            continue;
        }

        let mut port_modes = Vec::new();
        for port in &ports {
            let mode = match protocol_override.as_deref() {
                Some("http") => ProxyProtocol::Http,
                Some("tcp") => ProxyProtocol::Tcp,
                Some("auto" | "true") | None => guess_protocol(*port),
                Some(other) => {
                    eprintln!("[compose] unknown sanelens.proxy value '{other}' on {name}");
                    guess_protocol(*port)
                }
            };
            port_modes.push((*port, mode));
        }

        let app_name = format!("{name}-app");
        app_service_map.insert(app_name.clone(), name.clone());
        proxy_app_map.insert(name.clone(), app_name.clone());

        let original_ports = service.remove(Value::String("ports".to_string()));
        let original_expose = service.remove(Value::String("expose".to_string()));
        let original_container_name = service.remove(Value::String("container_name".to_string()));

        let mut app_service = service.clone();
        ensure_expose_ports(&mut app_service, &ports, original_expose.as_ref());
        add_label(&mut app_service, "sanelens.app", "true");
        add_label(&mut app_service, "sanelens.app.name", &name);
        add_run_labels(&mut app_service, &name, &run_labels);
        if config.enable_egress {
            ensure_env_var(
                &mut app_service,
                "HTTP_PROXY",
                "http://sanelens-egress-proxy:15001",
            );
            ensure_env_var(
                &mut app_service,
                "HTTPS_PROXY",
                "http://sanelens-egress-proxy:15001",
            );
            merge_env_var(&mut app_service, "NO_PROXY", &no_proxy_value);
        }

        let mut proxy_service = Mapping::new();
        proxy_service.insert(
            Value::String("image".to_string()),
            Value::String(config.envoy_image.clone()),
        );
        if config.disable_pods {
            add_envoy_entrypoint(&mut proxy_service);
        }
        if let Some(restart) = service.get(Value::String("restart".to_string())) {
            proxy_service.insert(Value::String("restart".to_string()), restart.clone());
        }
        if let Some(networks) = service.get(Value::String("networks".to_string())) {
            proxy_service.insert(Value::String("networks".to_string()), networks.clone());
        }
        let depends = build_proxy_depends_on(&app_name);
        proxy_service.insert(Value::String("depends_on".to_string()), depends);
        if let Some(ports_value) = original_ports.clone() {
            proxy_service.insert(Value::String("ports".to_string()), ports_value);
        }
        if let Some(container_name) = original_container_name {
            proxy_service.insert(Value::String("container_name".to_string()), container_name);
        }
        let expose_value = build_expose_value(&ports, original_expose.as_ref());
        if let Some(expose) = expose_value {
            proxy_service.insert(Value::String("expose".to_string()), expose);
        }
        let envoy_config = envoy_dir.join(format!("{name}.yaml"));
        let envoy_config_path = envoy_config.to_string_lossy();
        let tap_service_dir = tap_dir.join(&name);
        fs::create_dir_all(&tap_service_dir)
            .map_err(|err| format!("failed to create tap dir for {name}: {err}"))?;
        let tap_service_path = tap_service_dir.to_string_lossy();
        let volumes_value = Value::Sequence(vec![
            Value::String(format!("{envoy_config_path}:/etc/envoy/envoy.yaml:ro")),
            Value::String(format!("{tap_service_path}:/sanelens/tap")),
        ]);
        proxy_service.insert(Value::String("volumes".to_string()), volumes_value);
        add_label(&mut proxy_service, "sanelens.proxy", "true");
        add_label(&mut proxy_service, "sanelens.proxy.name", &name);
        add_run_labels(&mut proxy_service, &name, &run_labels);

        write_envoy_config(&envoy_dir, &name, &app_name, &port_modes)
            .map_err(|err| format!("failed to write envoy config: {err}"))?;

        new_services.insert(Value::String(name.clone()), Value::Mapping(proxy_service));
        new_services.insert(Value::String(app_name), Value::Mapping(app_service));
        proxy_services.insert(name);
    }

    if config.enable_egress {
        let egress_name = "sanelens-egress-proxy".to_string();
        let tap_service_dir = tap_dir.join(&egress_name);
        fs::create_dir_all(&tap_service_dir)
            .map_err(|err| format!("failed to create tap dir for {egress_name}: {err}"))?;
        let mut egress_config = build_egress_service(
            &config.envoy_image,
            &network_names,
            &envoy_dir.join("egress.yaml"),
            config.disable_pods,
            Some(&tap_service_dir),
        );
        if let Value::Mapping(map) = &mut egress_config {
            add_run_labels(map, &egress_name, &run_labels);
        }
        let egress_envoy = envoy_dir.join("egress.yaml");
        write_egress_envoy_config(&egress_envoy)
            .map_err(|err| format!("failed to write egress envoy config: {err}"))?;
        new_services.insert(Value::String(egress_name.clone()), egress_config);
        proxy_services.insert(egress_name);
    }

    for (_, value) in &mut new_services {
        let Value::Mapping(service) = value else {
            continue;
        };
        rewrite_depends_on_for_proxies(service, &proxy_app_map);
    }

    *services = new_services;

    let payload =
        serde_yaml::to_string(&doc).map_err(|err| format!("serialize compose failed: {err}"))?;
    fs::write(&derived_path, payload)
        .map_err(|err| format!("write derived compose failed: {err}"))?;

    Ok(DerivedCompose {
        path: derived_path,
        run_dir: out_dir,
        proxy_services,
        app_service_map,
        egress_proxy: if config.enable_egress {
            Some("sanelens-egress-proxy".to_string())
        } else {
            None
        },
    })
}

fn load_compose_doc(
    compose_path: &Path,
    project_name: &str,
    config: &DeriveConfig,
) -> Result<Value, String> {
    if config.compose_cmd.is_empty() {
        return Err("compose command is empty".to_string());
    }
    let mut cmd = config.compose_cmd.clone();
    let mut args = extract_compose_global_args(&config.compose_args);
    args.push("-p".to_string());
    args.push(project_name.to_string());
    if !config.compose_file_from_args {
        args.push("-f".to_string());
        args.push(compose_path.to_string_lossy().into_owned());
    }
    cmd.extend(args);
    cmd.push("config".to_string());

    let output = run_compose_output(&cmd).map_err(|err| format!("compose config failed: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        if stderr.is_empty() {
            return Err("compose config failed".to_string());
        }
        return Err(format!("compose config failed: {stderr}"));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let payload = stdout.trim();
    if payload.is_empty() {
        return Err("compose config returned empty output".to_string());
    }
    serde_yaml::from_str(payload).map_err(|err| format!("invalid compose config yaml: {err}"))
}

fn run_compose_output(cmd: &[String]) -> std::io::Result<std::process::Output> {
    let Some((program, args)) = cmd.split_first() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "empty command",
        ));
    };
    let mut command = Command::new(program);
    command
        .args(args)
        .env_remove("COMPOSE_PROJECT_NAME")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.output()
}

fn build_proxy_depends_on(app_name: &str) -> Value {
    let mut map = Mapping::new();
    map.insert(
        Value::String(app_name.to_string()),
        Value::Mapping(Mapping::new()),
    );
    Value::Mapping(map)
}

fn rewrite_depends_on_for_proxies(service: &mut Mapping, proxy_app_map: &HashMap<String, String>) {
    let depends_key = Value::String("depends_on".to_string());
    let Some(depends) = service.get_mut(&depends_key) else {
        return;
    };
    let Value::Mapping(depends_map) = depends else {
        return;
    };
    let mut replacements = Vec::new();
    for (key, value) in depends_map.iter() {
        let Some(service_name) = key.as_str() else {
            continue;
        };
        let Some(app_name) = proxy_app_map.get(service_name) else {
            continue;
        };
        if is_service_healthy_condition(value) {
            replacements.push((service_name.to_string(), app_name.clone(), value.clone()));
        }
    }
    for (old, new, config) in replacements {
        depends_map.remove(Value::String(old));
        depends_map.insert(Value::String(new), config);
    }
}

fn is_service_healthy_condition(value: &Value) -> bool {
    let Value::Mapping(map) = value else {
        return false;
    };
    map.get(Value::String("condition".to_string()))
        .and_then(|value| value.as_str())
        .is_some_and(|condition| condition == "service_healthy")
}

fn build_expose_value(ports: &[u16], original: Option<&Value>) -> Option<Value> {
    let mut items: Vec<Value> = Vec::new();
    for port in ports {
        items.push(Value::String(port.to_string()));
    }
    if let Some(Value::Sequence(entries)) = original {
        for entry in entries {
            items.push(entry.clone());
        }
    }
    if items.is_empty() {
        None
    } else {
        Some(Value::Sequence(items))
    }
}

fn ensure_expose_ports(service: &mut Mapping, ports: &[u16], original_expose: Option<&Value>) {
    let expose_value = build_expose_value(ports, original_expose);
    if let Some(value) = expose_value {
        service.insert(Value::String("expose".to_string()), value);
    }
}

fn read_proxy_protocol(service: &Mapping) -> Option<String> {
    let labels = service.get(Value::String("labels".to_string()));
    let key = "sanelens.proxy";
    match labels {
        Some(Value::Sequence(list)) => {
            list.iter()
                .filter_map(|entry| entry.as_str())
                .find_map(|entry| {
                    entry
                        .strip_prefix(&format!("{key}="))
                        .map(str::to_lowercase)
                })
        }
        Some(Value::Mapping(map)) => map
            .get(Value::String(key.to_string()))
            .and_then(|value| value.as_str())
            .map(str::to_lowercase),
        _ => None,
    }
}

fn add_label(service: &mut Mapping, key: &str, value: &str) {
    let labels_key = Value::String("labels".to_string());
    if key == STARTED_AT_LABEL {
        let label = Value::String(format!("{key}={value}"));
        match service.get_mut(&labels_key) {
            Some(Value::Sequence(list)) => {
                list.push(label);
            }
            Some(Value::Mapping(map)) => {
                let mut list = Vec::with_capacity(map.len() + 1);
                for (map_key, map_value) in map
                    .iter()
                    .filter_map(|(key, value)| key.as_str().map(|key| (key, value)))
                {
                    let map_value = label_value_string(map_value);
                    list.push(Value::String(format!("{map_key}={map_value}")));
                }
                list.push(label);
                service.insert(labels_key, Value::Sequence(list));
            }
            _ => {
                service.insert(labels_key, Value::Sequence(vec![label]));
            }
        }
        return;
    }
    match service.get_mut(&labels_key) {
        Some(Value::Mapping(map)) => {
            map.insert(
                Value::String(key.to_string()),
                Value::String(value.to_string()),
            );
        }
        Some(Value::Sequence(list)) => {
            list.push(Value::String(format!("{key}={value}")));
        }
        _ => {
            let mut map = Mapping::new();
            map.insert(
                Value::String(key.to_string()),
                Value::String(value.to_string()),
            );
            service.insert(labels_key, Value::Mapping(map));
        }
    }
}

fn label_value_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Null => "null".to_string(),
        other => serde_yaml::to_string(other)
            .unwrap_or_default()
            .trim()
            .to_string(),
    }
}

fn add_run_labels(service: &mut Mapping, service_name: &str, labels: &RunLabelContext<'_>) {
    add_label(service, RUN_ID_LABEL, labels.run_id);
    add_label(service, SERVICE_LABEL, service_name);
    add_label(service, COMPOSE_FILE_LABEL, labels.compose_file);
    add_label(service, DERIVED_COMPOSE_LABEL, labels.derived_compose);
    add_label(service, STARTED_AT_LABEL, labels.started_at);
    add_label(service, PROJECT_NAME_LABEL, labels.project_name);
}

fn ensure_env_var(service: &mut Mapping, key: &str, value: &str) {
    let env_key = Value::String("environment".to_string());
    match service.get_mut(&env_key) {
        Some(Value::Mapping(map)) => {
            map.entry(Value::String(key.to_string()))
                .or_insert(Value::String(value.to_string()));
        }
        Some(Value::Sequence(list)) => {
            if list.iter().any(|entry| {
                entry
                    .as_str()
                    .is_some_and(|item| item.starts_with(&format!("{key}=")))
            }) {
                return;
            }
            list.push(Value::String(format!("{key}={value}")));
        }
        _ => {
            let mut map = Mapping::new();
            map.insert(
                Value::String(key.to_string()),
                Value::String(value.to_string()),
            );
            service.insert(env_key, Value::Mapping(map));
        }
    }
}

fn merge_env_var(service: &mut Mapping, key: &str, value: &str) {
    let env_key = Value::String("environment".to_string());
    match service.get_mut(&env_key) {
        Some(Value::Mapping(map)) => {
            if let Some(existing) = map
                .get(Value::String(key.to_string()))
                .and_then(|value| value.as_str())
            {
                let merged = format!("{existing},{value}");
                map.insert(Value::String(key.to_string()), Value::String(merged));
            } else {
                map.insert(
                    Value::String(key.to_string()),
                    Value::String(value.to_string()),
                );
            }
        }
        Some(Value::Sequence(list)) => {
            let mut updated = false;
            let prefix = format!("{key}=");
            for entry in list.iter_mut() {
                let Some(item) = entry.as_str().filter(|item| item.starts_with(&prefix)) else {
                    continue;
                };
                let merged = format!("{item},{value}");
                *entry = Value::String(merged);
                updated = true;
                break;
            }
            if !updated {
                list.push(Value::String(format!("{key}={value}")));
            }
        }
        _ => {
            let mut map = Mapping::new();
            map.insert(
                Value::String(key.to_string()),
                Value::String(value.to_string()),
            );
            service.insert(env_key, Value::Mapping(map));
        }
    }
}

fn get_string(map: &Mapping, key: &str) -> Option<String> {
    map.get(Value::String(key.to_string()))
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
}

fn extract_ports(service: &Mapping) -> Vec<u16> {
    let mut ports = Vec::new();
    if let Some(Value::Sequence(entries)) = service.get(Value::String("ports".to_string())) {
        for entry in entries {
            let port = match entry {
                Value::String(value) => parse_container_port(value),
                Value::Mapping(map) => map
                    .get(Value::String("target".to_string()))
                    .and_then(value_to_u16),
                _ => None,
            };
            if let Some(port) = port {
                ports.push(port);
            }
        }
    }
    if let Some(Value::Sequence(entries)) = service.get(Value::String("expose".to_string())) {
        for entry in entries {
            if let Some(port) = value_to_u16(entry) {
                ports.push(port);
            }
        }
    }
    ports.sort_unstable();
    ports.dedup();
    ports
}

fn value_to_u16(value: &Value) -> Option<u16> {
    match value {
        Value::Number(num) => num.as_u64().and_then(|v| u16::try_from(v).ok()),
        Value::String(value) => {
            let token = value.split('/').next().unwrap_or(value);
            parse_port_token(token)
        }
        _ => None,
    }
}

fn parse_container_port(entry: &str) -> Option<u16> {
    let entry = entry.split('/').next().unwrap_or(entry).trim();
    if entry.is_empty() {
        return None;
    }
    let port_str = find_container_port_separator(entry)
        .map_or(entry, |idx| entry.get(idx + 1..).unwrap_or(""));
    let port_str = port_str.trim();
    if port_str.is_empty() {
        return None;
    }
    parse_port_token(port_str)
}

fn find_container_port_separator(entry: &str) -> Option<usize> {
    let mut in_env = false;
    let mut in_brackets = false;
    let mut last_colon = None;
    let mut chars = entry.char_indices().peekable();
    while let Some((idx, ch)) = chars.next() {
        match ch {
            '[' if !in_env => {
                in_brackets = true;
            }
            ']' if !in_env => {
                in_brackets = false;
            }
            '$' if !in_env => {
                if matches!(chars.peek(), Some((_, '{'))) {
                    in_env = true;
                }
            }
            '}' if in_env => {
                in_env = false;
            }
            ':' if !in_env && !in_brackets => {
                last_colon = Some(idx);
            }
            _ => {}
        }
    }
    last_colon
}

fn parse_port_token(token: &str) -> Option<u16> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    if let Ok(port) = token.parse::<u16>() {
        return Some(port);
    }
    let inner = token
        .strip_prefix("${")
        .and_then(|rest| rest.strip_suffix('}'))?;
    let default = if let Some((_, default)) = inner.split_once(":-") {
        default
    } else if let Some((_, default)) = inner.split_once('-') {
        default
    } else {
        return None;
    };
    let default = default.trim().trim_matches('"').trim_matches('\'');
    parse_port_token(default)
}

fn guess_protocol(port: u16) -> ProxyProtocol {
    const HTTP_PORTS: [u16; 12] = [
        80, 443, 3000, 3001, 3002, 5173, 8000, 8080, 8100, 9000, 10000, 15672,
    ];
    if HTTP_PORTS.contains(&port) {
        ProxyProtocol::Http
    } else {
        ProxyProtocol::Tcp
    }
}

#[cfg(test)]
#[allow(clippy::literal_string_with_formatting_args)]
mod tests {
    use super::parse_container_port;

    #[test]
    fn parse_container_port_plain() {
        assert_eq!(parse_container_port("8080"), Some(8080));
        assert_eq!(parse_container_port("8080/tcp"), Some(8080));
    }

    #[test]
    fn parse_container_port_host_mapping() {
        assert_eq!(parse_container_port("127.0.0.1:3000:80"), Some(80));
        assert_eq!(parse_container_port("0.0.0.0:3000:8080/udp"), Some(8080));
    }

    #[test]
    fn parse_container_port_env_defaults() {
        assert_eq!(
            parse_container_port("${HOST_PORT:-8080}:${PORT:-3000}"),
            Some(3000)
        );
        assert_eq!(parse_container_port("${PORT:-3000}"), Some(3000));
        assert_eq!(parse_container_port("${PORT-3000}"), Some(3000));
    }

    #[test]
    fn parse_container_port_ipv6() {
        assert_eq!(parse_container_port("[::1]:3000:80"), Some(80));
        assert_eq!(
            parse_container_port("[::1]:${HOST_PORT:-3000}:${PORT:-80}"),
            Some(80)
        );
    }
}

fn build_egress_service(
    envoy_image: &str,
    networks: &[String],
    config_path: &Path,
    override_entrypoint: bool,
    tap_dir: Option<&Path>,
) -> Value {
    let mut map = Mapping::new();
    map.insert(
        Value::String("image".to_string()),
        Value::String(envoy_image.to_string()),
    );
    if override_entrypoint {
        add_envoy_entrypoint(&mut map);
    }
    let config_path_display = config_path.to_string_lossy();
    let mut volumes = vec![Value::String(format!(
        "{config_path_display}:/etc/envoy/envoy.yaml:ro"
    ))];
    if let Some(tap_dir) = tap_dir {
        let tap_path = tap_dir.to_string_lossy();
        volumes.push(Value::String(format!("{tap_path}:/sanelens/tap")));
    }
    map.insert(
        Value::String("volumes".to_string()),
        Value::Sequence(volumes),
    );
    if !networks.is_empty() {
        let list = networks
            .iter()
            .map(|name| Value::String(name.clone()))
            .collect();
        map.insert(Value::String("networks".to_string()), Value::Sequence(list));
    }
    add_label(&mut map, "sanelens.proxy", "true");
    add_label(&mut map, "sanelens.proxy.egress", "true");
    Value::Mapping(map)
}

fn add_envoy_entrypoint(service: &mut Mapping) {
    service.insert(
        Value::String("entrypoint".to_string()),
        Value::Sequence(vec![Value::String("envoy".to_string())]),
    );
    service.insert(
        Value::String("command".to_string()),
        Value::Sequence(vec![
            Value::String("-c".to_string()),
            Value::String("/etc/envoy/envoy.yaml".to_string()),
        ]),
    );
}

fn collect_network_names(doc: &Value) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(Value::Mapping(map)) = doc.get("networks") {
        for key in map.keys() {
            if let Some(name) = key.as_str() {
                names.push(name.to_string());
            }
        }
    }
    names
}

fn collect_service_names(doc: &Value) -> Result<Vec<String>, String> {
    let Some(Value::Mapping(services)) = doc.get("services") else {
        return Err("compose file missing services".to_string());
    };
    let mut names: Vec<String> = services
        .keys()
        .filter_map(|key| key.as_str().map(ToString::to_string))
        .collect();
    names.sort();
    Ok(names)
}

fn set_compose_name(doc: &mut Value, project_name: &str) {
    let Value::Mapping(map) = doc else {
        return;
    };
    map.insert(
        Value::String("name".to_string()),
        Value::String(project_name.to_string()),
    );
}

fn rewrite_top_level_paths(doc: &mut Value, base_dir: &Path) {
    if let Some(Value::Mapping(map)) = doc.get_mut("configs") {
        rewrite_named_file_entries(map, base_dir);
    }
    if let Some(Value::Mapping(map)) = doc.get_mut("secrets") {
        rewrite_named_file_entries(map, base_dir);
    }
}

fn disable_podman_pods(doc: &mut Value) {
    let Value::Mapping(map) = doc else {
        return;
    };
    let key = Value::String("x-podman".to_string());
    let in_pod_key = Value::String("in_pod".to_string());
    if let Some(Value::Mapping(x_podman)) = map.get_mut(&key) {
        x_podman.insert(in_pod_key, Value::Bool(false));
    } else {
        let mut x_podman = Mapping::new();
        x_podman.insert(in_pod_key, Value::Bool(false));
        map.insert(key, Value::Mapping(x_podman));
    }
}

fn rewrite_string_value(value: &mut String, base_dir: &Path, kind: PathKind) {
    if let Some(updated) = rewrite_path_value(value, base_dir, kind) {
        *value = updated;
    }
}

fn rewrite_named_file_entries(map: &mut Mapping, base_dir: &Path) {
    for (_, entry) in map.iter_mut() {
        rewrite_named_file_entry(entry, base_dir);
    }
}

fn rewrite_named_file_entry(entry: &mut Value, base_dir: &Path) {
    let Value::Mapping(def) = entry else {
        return;
    };
    let Some(Value::String(file)) = def.get_mut(Value::String("file".to_string())) else {
        return;
    };
    rewrite_string_value(file, base_dir, PathKind::File);
}

fn rewrite_service_paths(service: &mut Mapping, base_dir: &Path) {
    rewrite_build(service, base_dir);
    rewrite_env_files(service, base_dir);
    rewrite_volumes(service, base_dir);
    rewrite_extends(service, base_dir);
}

fn rewrite_build(service: &mut Mapping, base_dir: &Path) {
    let Some(value) = service.get_mut(Value::String("build".to_string())) else {
        return;
    };
    match value {
        Value::String(context) => {
            rewrite_string_value(context, base_dir, PathKind::Dir);
        }
        Value::Mapping(map) => {
            if let Some(Value::String(context)) = map.get_mut(Value::String("context".to_string()))
            {
                rewrite_string_value(context, base_dir, PathKind::Dir);
            }
            if let Some(Value::Mapping(additional)) =
                map.get_mut(Value::String("additional_contexts".to_string()))
            {
                rewrite_additional_contexts(additional, base_dir);
            }
        }
        _ => {}
    }
}

fn rewrite_additional_contexts(additional: &mut Mapping, base_dir: &Path) {
    for (_, ctx_value) in additional.iter_mut() {
        let Value::String(ctx) = ctx_value else {
            continue;
        };
        rewrite_string_value(ctx, base_dir, PathKind::Dir);
    }
}

fn rewrite_env_files(service: &mut Mapping, base_dir: &Path) {
    let Some(value) = service.get_mut(Value::String("env_file".to_string())) else {
        return;
    };
    match value {
        Value::String(path) => rewrite_string_value(path, base_dir, PathKind::File),
        Value::Sequence(entries) => {
            for entry in entries {
                let Value::String(path) = entry else {
                    continue;
                };
                rewrite_string_value(path, base_dir, PathKind::File);
            }
        }
        _ => {}
    }
}

fn rewrite_extends(service: &mut Mapping, base_dir: &Path) {
    let Some(Value::Mapping(map)) = service.get_mut(Value::String("extends".to_string())) else {
        return;
    };
    let Some(Value::String(file)) = map.get_mut(Value::String("file".to_string())) else {
        return;
    };
    rewrite_string_value(file, base_dir, PathKind::File);
}

fn rewrite_volumes(service: &mut Mapping, base_dir: &Path) {
    let volumes_key = Value::String("volumes".to_string());
    let Some(Value::Sequence(entries)) = service.get_mut(&volumes_key) else {
        return;
    };
    for entry in entries {
        match entry {
            Value::String(value) => {
                if let Some(updated) = rewrite_volume_short(value, base_dir) {
                    *value = updated;
                }
            }
            Value::Mapping(map) => {
                let kind = match map.get(Value::String("type".to_string())) {
                    Some(Value::String(kind)) if kind == "bind" => PathKind::Dir,
                    Some(Value::String(kind)) if kind == "volume" => PathKind::Volume,
                    _ => PathKind::Unknown,
                };
                let Some(Value::String(source)) = map.get_mut(Value::String("source".to_string()))
                else {
                    continue;
                };
                rewrite_string_value(source, base_dir, kind);
            }
            _ => {}
        }
    }
}

#[derive(Clone, Copy)]
enum PathKind {
    Dir,
    File,
    Volume,
    Unknown,
}

fn rewrite_volume_short(value: &str, base_dir: &Path) -> Option<String> {
    let (source, target, mode) = split_volume_entry(value)?;
    if source.is_empty() {
        return None;
    }
    let kind = if is_probably_path(&source) {
        PathKind::Dir
    } else {
        PathKind::Volume
    };
    if let Some(updated) = rewrite_path_value(&source, base_dir, kind) {
        let mut rebuilt = format!("{updated}:{target}");
        if let Some(mode) = mode {
            rebuilt.push(':');
            rebuilt.push_str(&mode);
        }
        return Some(rebuilt);
    }
    None
}

fn split_volume_entry(value: &str) -> Option<(String, String, Option<String>)> {
    let parts: Vec<&str> = value.split(':').collect();
    if parts.len() < 2 {
        return None;
    }
    let mut index = 1;
    let first = parts.first().copied().unwrap_or("");
    let mut source = first.to_string();
    if parts.len() >= 3
        && first.len() == 1
        && parts
            .get(1)
            .is_some_and(|part| part.starts_with('\\') || part.starts_with('/'))
    {
        let source_drive = first;
        let source_path = parts.get(1).copied().unwrap_or("");
        source = format!("{source_drive}:{source_path}");
        index = 2;
    }
    let remaining = parts.get(index..)?;
    let target = remaining.first().copied()?.to_string();
    let mode = if remaining.len() > 1 {
        let extra: Vec<&str> = remaining.iter().skip(1).copied().collect();
        Some(extra.join(":"))
    } else {
        None
    };
    Some((source, target, mode))
}

fn rewrite_path_value(value: &str, base_dir: &Path, kind: PathKind) -> Option<String> {
    if value.contains("${") || value.contains('$') {
        if let Some(updated) = rewrite_default_expr(value, base_dir, kind) {
            return Some(updated);
        }
        return None;
    }
    if is_uri_like(value) {
        return None;
    }
    let path = Path::new(value);
    if path.is_absolute() {
        return None;
    }
    if matches!(kind, PathKind::Volume | PathKind::Unknown) && !is_probably_path(value) {
        return None;
    }
    let expanded = expand_tilde(value);
    let path = Path::new(&expanded);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    };
    Some(absolute.to_string_lossy().into_owned())
}

fn rewrite_default_expr(value: &str, base_dir: &Path, kind: PathKind) -> Option<String> {
    let trimmed = value.trim();
    let inner = trimmed
        .strip_prefix("${")
        .and_then(|rest| rest.strip_suffix('}'))?;
    let (var, default, op) = if let Some((var, default)) = inner.split_once(":-") {
        (var, default, ":-")
    } else if let Some((var, default)) = inner.split_once('-') {
        (var, default, "-")
    } else {
        return None;
    };
    let default = default.trim();
    if default.is_empty() {
        return None;
    }
    if matches!(kind, PathKind::Volume | PathKind::Unknown) && !is_probably_path(default) {
        return None;
    }
    let default_rewrite = rewrite_path_value(default, base_dir, PathKind::Unknown)?;
    let var_name = var.trim();
    Some(format!("${{{var_name}{op}{default_rewrite}}}"))
}

fn expand_tilde(value: &str) -> String {
    if let Some(rest) = value.strip_prefix("~/") {
        if let Ok(home) = env::var("HOME") {
            return format!("{home}/{rest}");
        }
    }
    value.to_string()
}

fn is_probably_path(value: &str) -> bool {
    if value == "." || value == ".." {
        return true;
    }
    value.starts_with("./")
        || value.starts_with("../")
        || value.starts_with('/')
        || value.starts_with('~')
        || value.contains('/')
        || value.contains('\\')
}

fn to_absolute_path(path: &str) -> Result<PathBuf, String> {
    let candidate = PathBuf::from(path);
    if candidate.is_absolute() {
        return Ok(candidate);
    }
    let cwd = env::current_dir().map_err(|err| err.to_string())?;
    Ok(cwd.join(candidate))
}

fn is_uri_like(value: &str) -> bool {
    let lower = value.to_lowercase();
    lower.contains("://") || lower.starts_with("git@")
}

fn write_envoy_config(
    envoy_dir: &Path,
    service_name: &str,
    app_name: &str,
    ports: &[(u16, ProxyProtocol)],
) -> Result<(), String> {
    let mut body = String::new();
    body.push_str("static_resources:\n  listeners:\n");
    for (port, mode) in ports {
        match mode {
            ProxyProtocol::Http => {
                body.push_str(&http_listener_block(service_name, app_name, *port));
            }
            ProxyProtocol::Tcp => {
                body.push_str(&tcp_listener_block(service_name, app_name, *port));
            }
        }
    }
    body.push_str("  clusters:\n");
    for (port, _) in ports {
        body.push_str(&cluster_block(app_name, *port));
    }
    body.push_str("admin:\n  access_log_path: /tmp/envoy_admin.log\n  address:\n    socket_address:\n      address: 0.0.0.0\n      port_value: 9901\n");

    let path = envoy_dir.join(format!("{service_name}.yaml"));
    fs::write(path, body).map_err(|err| err.to_string())
}

const EGRESS_ENVOY_CONFIG: &str = r#"static_resources:
  listeners:
  - name: egress_listener
    address:
      socket_address:
        address: 0.0.0.0
        port_value: 15001
    filter_chains:
    - filters:
      - name: envoy.filters.network.http_connection_manager
        typed_config:
          "@type": type.googleapis.com/envoy.extensions.filters.network.http_connection_manager.v3.HttpConnectionManager
          stat_prefix: egress_http
          route_config:
            name: egress_route
            virtual_hosts:
            - name: default
              domains: ["*"]
              routes:
              - match:
                  prefix: "/"
                route:
                  cluster: egress_cluster
                  timeout: 0s
          http_filters:
          - name: envoy.filters.http.dynamic_forward_proxy
            typed_config:
              "@type": type.googleapis.com/envoy.extensions.filters.http.dynamic_forward_proxy.v3.FilterConfig
              dns_cache_config:
                name: egress_cache
                dns_lookup_family: V4_ONLY
          - name: envoy.filters.http.tap
            typed_config:
              "@type": type.googleapis.com/envoy.extensions.filters.http.tap.v3.Tap
              common_config:
                static_config:
                  match_config:
                    any_match: true
                  output_config:
                    max_buffered_rx_bytes: 10485760
                    max_buffered_tx_bytes: 10485760
                    sinks:
                    - format: JSON_BODY_AS_STRING
                      file_per_tap:
                        path_prefix: /sanelens/tap/trace
          - name: envoy.filters.http.router
            typed_config:
              "@type": type.googleapis.com/envoy.extensions.filters.http.router.v3.Router
          access_log:
          - name: envoy.access_loggers.stdout
            typed_config:
              "@type": type.googleapis.com/envoy.extensions.access_loggers.stream.v3.StdoutAccessLog
              log_format:
                json_format:
                  timestamp: "%START_TIME%"
                  method: "%REQ(:METHOD)%"
                  path: "%REQ(X-ENVOY-ORIGINAL-PATH?:PATH)%"
                  authority: "%REQ(:AUTHORITY)%"
                  request_id: "%REQ(X-REQUEST-ID)%"
                  request_user_agent: "%REQ(USER-AGENT)%"
                  request_content_type: "%REQ(CONTENT-TYPE)%"
                  request_accept: "%REQ(ACCEPT)%"
                  request_body: "%DYNAMIC_METADATA(sanelens:request_body)%"
                  request_forwarded_for: "%REQ(X-FORWARDED-FOR)%"
                  request_forwarded_proto: "%REQ(X-FORWARDED-PROTO)%"
                  response_content_type: "%RESP(CONTENT-TYPE)%"
                  response_content_length: "%RESP(CONTENT-LENGTH)%"
                  response_body: "%DYNAMIC_METADATA(sanelens:response_body)%"
                  response_code: "%RESPONSE_CODE%"
                  duration_ms: "%DURATION%"
                  downstream_remote_address: "%DOWNSTREAM_REMOTE_ADDRESS%"
                  upstream_host: "%UPSTREAM_HOST%"
                  bytes_received: "%BYTES_RECEIVED%"
                  bytes_sent: "%BYTES_SENT%"
  clusters:
  - name: egress_cluster
    connect_timeout: 5s
    lb_policy: CLUSTER_PROVIDED
    cluster_type:
      name: envoy.clusters.dynamic_forward_proxy
      typed_config:
        "@type": type.googleapis.com/envoy.extensions.clusters.dynamic_forward_proxy.v3.ClusterConfig
        dns_cache_config:
          name: egress_cache
          dns_lookup_family: V4_ONLY
admin:
  access_log_path: /tmp/envoy_admin.log
  address:
    socket_address:
      address: 0.0.0.0
      port_value: 9901
"#;
fn write_egress_envoy_config(path: &Path) -> Result<(), String> {
    fs::write(path, EGRESS_ENVOY_CONFIG).map_err(|err| err.to_string())
}

#[allow(clippy::too_many_lines)]
fn http_listener_block(service_name: &str, app_name: &str, port: u16) -> String {
    format!(
        r#"  - name: {service_name}_listener_{port}
    address:
      socket_address:
        address: 0.0.0.0
        port_value: {port}
    filter_chains:
    - filters:
      - name: envoy.filters.network.http_connection_manager
        typed_config:
          "@type": type.googleapis.com/envoy.extensions.filters.network.http_connection_manager.v3.HttpConnectionManager
          stat_prefix: ingress_http_{port}
          codec_type: AUTO
          route_config:
            name: route_{port}
            virtual_hosts:
            - name: backend
              domains: ["*"]
              routes:
              - match:
                  prefix: "/"
                route:
                  cluster: {app_name}_{port}
          http_filters:
          - name: envoy.filters.http.tap
            typed_config:
              "@type": type.googleapis.com/envoy.extensions.filters.http.tap.v3.Tap
              common_config:
                static_config:
                  match_config:
                    any_match: true
                  output_config:
                    max_buffered_rx_bytes: 10485760
                    max_buffered_tx_bytes: 10485760
                    sinks:
                    - format: JSON_BODY_AS_STRING
                      file_per_tap:
                        path_prefix: /sanelens/tap/trace
          - name: envoy.filters.http.router
            typed_config:
              "@type": type.googleapis.com/envoy.extensions.filters.http.router.v3.Router
          access_log:
          - name: envoy.access_loggers.stdout
            typed_config:
              "@type": type.googleapis.com/envoy.extensions.access_loggers.stream.v3.StdoutAccessLog
              log_format:
                json_format:
                  timestamp: "%START_TIME%"
                  method: "%REQ(:METHOD)%"
                  path: "%REQ(X-ENVOY-ORIGINAL-PATH?:PATH)%"
                  protocol: "%PROTOCOL%"
                  response_code: "%RESPONSE_CODE%"
                  duration_ms: "%DURATION%"
                  downstream_remote_address: "%DOWNSTREAM_REMOTE_ADDRESS%"
                  upstream_host: "%UPSTREAM_HOST%"
                  bytes_received: "%BYTES_RECEIVED%"
                  bytes_sent: "%BYTES_SENT%"
                  request_id: "%REQ(X-REQUEST-ID)%"
                  request_user_agent: "%REQ(USER-AGENT)%"
                  request_content_type: "%REQ(CONTENT-TYPE)%"
                  request_accept: "%REQ(ACCEPT)%"
                  request_body: "%DYNAMIC_METADATA(sanelens:request_body)%"
                  request_forwarded_for: "%REQ(X-FORWARDED-FOR)%"
                  request_forwarded_proto: "%REQ(X-FORWARDED-PROTO)%"
                  response_content_type: "%RESP(CONTENT-TYPE)%"
                  response_content_length: "%RESP(CONTENT-LENGTH)%"
                  response_body: "%DYNAMIC_METADATA(sanelens:response_body)%"
"#,
    )
}

fn tcp_listener_block(service_name: &str, app_name: &str, port: u16) -> String {
    format!(
        "  - name: {service_name}_tcp_listener_{port}\n    address:\n      socket_address:\n        address: 0.0.0.0\n        port_value: {port}\n    filter_chains:\n    - filters:\n      - name: envoy.filters.network.tcp_proxy\n        typed_config:\n          \"@type\": type.googleapis.com/envoy.extensions.filters.network.tcp_proxy.v3.TcpProxy\n          stat_prefix: tcp_{port}\n          cluster: {app_name}_{port}\n          access_log:\n          - name: envoy.access_loggers.stdout\n            typed_config:\n              \"@type\": type.googleapis.com/envoy.extensions.access_loggers.stream.v3.StdoutAccessLog\n              log_format:\n                json_format:\n                  timestamp: \"%START_TIME%\"\n                  duration_ms: \"%DURATION%\"\n                  downstream_remote_address: \"%DOWNSTREAM_REMOTE_ADDRESS%\"\n                  upstream_host: \"%UPSTREAM_HOST%\"\n                  bytes_received: \"%BYTES_RECEIVED%\"\n                  bytes_sent: \"%BYTES_SENT%\"\n",
    )
}

fn cluster_block(app_name: &str, port: u16) -> String {
    format!(
        "  - name: {app_name}_{port}\n    connect_timeout: 2s\n    type: STRICT_DNS\n    lb_policy: ROUND_ROBIN\n    load_assignment:\n      cluster_name: {app_name}_{port}\n      endpoints:\n      - lb_endpoints:\n        - endpoint:\n            address:\n              socket_address:\n                address: {app_name}\n                port_value: {port}\n",
    )
}
