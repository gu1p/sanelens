use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;

use crate::domain::ServiceInfo;

pub(crate) fn build_service_info(compose_file: &str) -> Vec<ServiceInfo> {
    let (services, ports_by_service) = parse_compose_services_and_ports(compose_file);
    let mut info = Vec::new();
    for name in services {
        let endpoints: Vec<String> = ports_by_service
            .get(&name)
            .map(|ports| {
                ports
                    .iter()
                    .map(|port| format!("http://localhost:{}", port))
                    .collect()
            })
            .unwrap_or_default();
        info.push(ServiceInfo {
            name: name.clone(),
            endpoint: endpoints.get(0).cloned(),
            exposed: !endpoints.is_empty(),
            endpoints,
        });
    }
    info
}

fn parse_compose_services_and_ports(
    compose_file: &str,
) -> (Vec<String>, HashMap<String, Vec<String>>) {
    let contents = match fs::read_to_string(compose_file) {
        Ok(contents) => contents,
        Err(_) => return (Vec::new(), HashMap::new()),
    };
    let doc: serde_yaml::Value = match serde_yaml::from_str(&contents) {
        Ok(doc) => doc,
        Err(_) => return (Vec::new(), HashMap::new()),
    };
    let services_val = match doc.get("services") {
        Some(val) => val,
        None => return (Vec::new(), HashMap::new()),
    };
    let services_map = match services_val.as_mapping() {
        Some(map) => map,
        None => return (Vec::new(), HashMap::new()),
    };

    let mut services = Vec::new();
    let mut ports_by_service: HashMap<String, Vec<String>> = HashMap::new();

    for (name_val, service_val) in services_map {
        let name = match name_val.as_str() {
            Some(name) => name.to_string(),
            None => continue,
        };
        services.push(name.clone());
        let mut ports = Vec::new();
        if let Some(service_map) = service_val.as_mapping() {
            if let Some(ports_val) =
                service_map.get(&serde_yaml::Value::String("ports".to_string()))
            {
                if let Some(list) = ports_val.as_sequence() {
                    for entry in list {
                        match entry {
                            serde_yaml::Value::String(value) => {
                                if let Some(host_port) = parse_port_short(value) {
                                    if let Some(port) = resolve_host_port(&host_port) {
                                        ports.push(port);
                                    }
                                }
                            }
                            serde_yaml::Value::Mapping(map) => {
                                if let Some(value) =
                                    map.get(&serde_yaml::Value::String("published".to_string()))
                                {
                                    if let Some(raw) = yaml_value_to_string(value) {
                                        if let Some(port) = resolve_host_port(&raw) {
                                            ports.push(port);
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        let mut seen = HashSet::new();
        let mut unique = Vec::new();
        for port in ports {
            if seen.insert(port.clone()) {
                unique.push(port);
            }
        }
        ports_by_service.insert(name, unique);
    }

    (services, ports_by_service)
}

fn yaml_value_to_string(value: &serde_yaml::Value) -> Option<String> {
    match value {
        serde_yaml::Value::String(value) => Some(value.clone()),
        serde_yaml::Value::Number(value) => Some(value.to_string()),
        serde_yaml::Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn strip_quotes(value: &str) -> &str {
    if let Some(stripped) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
        return stripped;
    }
    if let Some(stripped) = value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
        return stripped;
    }
    value
}

fn resolve_env_value(raw_value: &str) -> String {
    let value = strip_quotes(raw_value.trim());
    if value.starts_with("${") && value.ends_with('}') {
        let inner = &value[2..value.len() - 1];
        if let Some((var, default)) = inner.split_once(":-") {
            return env::var(var).unwrap_or_else(|_| default.to_string());
        }
        return env::var(inner).unwrap_or_default();
    }
    if let Some(var) = value.strip_prefix('$') {
        return env::var(var).unwrap_or_default();
    }
    value.to_string()
}

fn parse_port_short(value: &str) -> Option<String> {
    let entry = strip_quotes(value.trim());
    if entry.is_empty() {
        return None;
    }
    let entry = entry.split('/').next().unwrap_or(entry);
    let parts: Vec<&str> = entry.split(':').collect();
    if parts.len() == 1 {
        return None;
    }
    if parts.len() >= 3 {
        let first = parts[0].trim();
        if first.contains('.') || first == "localhost" || first == "0.0.0.0" {
            return Some(parts[1].trim().to_string());
        }
        return Some(first.to_string());
    }
    Some(parts[0].trim().to_string())
}

fn resolve_host_port(raw_port: &str) -> Option<String> {
    let value = resolve_env_value(raw_port).trim().to_string();
    if value.is_empty() || value == "0" {
        return None;
    }
    if value.chars().all(|c| c.is_ascii_digit()) {
        return Some(value);
    }
    None
}
