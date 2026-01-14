use std::env;

use crate::domain::EngineKind;

pub fn extract_engine_arg(args: &[String]) -> Result<(Vec<String>, Option<EngineKind>), String> {
    let mut updated = Vec::with_capacity(args.len());
    let mut selected = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--" {
            updated.push(arg.clone());
            updated.extend(iter.cloned());
            break;
        }
        if arg == "--engine" {
            let value = iter.next().map(String::as_str);
            selected = Some(parse_engine_kind(value)?);
            continue;
        }
        if let Some(value) = arg.strip_prefix("--engine=") {
            selected = Some(parse_engine_kind(Some(value))?);
            continue;
        }
        updated.push(arg.clone());
    }
    Ok((updated, selected))
}

pub fn extract_traffic_arg(args: &[String]) -> (Vec<String>, Option<bool>) {
    let mut updated = Vec::with_capacity(args.len());
    let mut override_value = None;
    for arg in args {
        if arg == "--traffic" || arg == "--comms" {
            override_value = Some(true);
            continue;
        }
        if arg == "--no-traffic" {
            override_value = Some(false);
            continue;
        }
        if let Some(value) = arg.strip_prefix("--traffic=") {
            let value = value.to_lowercase();
            if value == "0" || value == "false" || value == "no" {
                override_value = Some(false);
            } else {
                override_value = Some(true);
            }
            continue;
        }
        updated.push(arg.clone());
    }
    (updated, override_value)
}

pub fn strip_project_name_args(args: &[String]) -> Vec<String> {
    let mut updated = Vec::with_capacity(args.len());
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--" {
            updated.push(arg.clone());
            updated.extend(iter.cloned());
            break;
        }
        if arg == "-p" || arg == "--project-name" {
            let _ = iter.next();
            continue;
        }
        if arg.starts_with("--project-name=") || arg.starts_with("-p=") {
            continue;
        }
        updated.push(arg.clone());
    }
    updated
}

pub fn has_flag(args: &[String], names: &[&str]) -> bool {
    for arg in args {
        for name in names {
            if arg == name {
                return true;
            }
            let Some(value) = arg.strip_prefix(&format!("{name}=")) else {
                continue;
            };
            let value = value.to_lowercase();
            if is_falsey(&value) {
                break;
            }
            return true;
        }
    }
    false
}

pub fn extract_subcommand(args: &[String]) -> Option<String> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--" {
            return iter.next().cloned();
        }
        if arg.starts_with('-') {
            if arg.contains('=') {
                continue;
            }
            if option_takes_value(arg) {
                let _ = iter.next();
            }
            continue;
        }
        return Some(arg.clone());
    }
    None
}

fn option_takes_value(arg: &str) -> bool {
    matches!(
        arg,
        "-f" | "--file"
            | "-p"
            | "--project-name"
            | "--project-directory"
            | "--env-file"
            | "--profile"
            | "--ansi"
            | "--progress"
            | "--log-level"
            | "-H"
            | "--host"
            | "--context"
    )
}

pub fn take_flag(args: &[String], name: &str) -> (Vec<String>, bool) {
    let mut updated = Vec::with_capacity(args.len());
    let mut enabled = false;
    let prefix = format!("{name}=");
    for arg in args {
        if arg == name {
            enabled = true;
            continue;
        }
        if let Some(value) = arg.strip_prefix(&prefix) {
            let value = value.to_lowercase();
            if value == "0" || value == "false" || value == "no" {
                continue;
            }
            enabled = true;
            continue;
        }
        updated.push(arg.clone());
    }
    (updated, enabled)
}

pub fn extract_compose_file_arg(args: &[String]) -> Option<String> {
    let mut found = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "-f" || arg == "--file" {
            if let Some(value) = iter.next() {
                found = Some(value.clone());
            }
            continue;
        }
        if let Some(value) = arg.strip_prefix("--file=") {
            found = Some(value.to_string());
            continue;
        }
        if let Some(value) = arg.strip_prefix("-f=") {
            found = Some(value.to_string());
        }
    }
    found
}

pub fn extract_compose_global_args(args: &[String]) -> Vec<String> {
    let mut extracted = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--" {
            break;
        }
        if matches!(
            arg.as_str(),
            "-f" | "--file" | "--env-file" | "--project-directory" | "--profile" | "--in-pod"
        ) {
            if let Some(value) = iter.next() {
                extracted.push(arg.clone());
                extracted.push(value.clone());
            }
            continue;
        }
        if arg.starts_with("--file=")
            || arg.starts_with("-f=")
            || arg.starts_with("--env-file=")
            || arg.starts_with("--project-directory=")
            || arg.starts_with("--profile=")
            || arg.starts_with("--in-pod=")
        {
            extracted.push(arg.clone());
        }
    }
    extracted
}

pub fn strip_compose_file_args(args: &[String]) -> Vec<String> {
    let mut updated = Vec::with_capacity(args.len());
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "-f" || arg == "--file" {
            let _ = iter.next();
            continue;
        }
        if arg.starts_with("--file=") || arg.starts_with("-f=") {
            continue;
        }
        updated.push(arg.clone());
    }
    updated
}

pub fn first_compose_file(value: &str) -> Option<String> {
    let separator = if cfg!(windows) { ';' } else { ':' };
    value
        .split(separator)
        .map(str::trim)
        .find(|entry| !entry.is_empty())
        .map(ToString::to_string)
}

pub fn insert_after(args: &[String], token: &str, new_arg: &str) -> Vec<String> {
    let mut updated = Vec::new();
    let mut inserted = false;
    for arg in args {
        updated.push(arg.clone());
        if !inserted && arg == token {
            updated.push(new_arg.to_string());
            inserted = true;
        }
    }
    if !inserted {
        updated.push(new_arg.to_string());
    }
    updated
}

pub fn is_env_false(name: &str) -> bool {
    env::var(name).is_ok_and(|value| matches!(value.to_lowercase().as_str(), "0" | "false" | "no"))
}

pub fn is_env_truthy(name: &str) -> bool {
    env::var(name).is_ok_and(|value| matches!(value.to_lowercase().as_str(), "1" | "true" | "yes"))
}

fn parse_engine_kind(value: Option<&str>) -> Result<EngineKind, String> {
    let raw =
        value.ok_or_else(|| "--engine requires a value of 'podman' or 'docker'.".to_string())?;
    match raw.to_lowercase().as_str() {
        "podman" => Ok(EngineKind::Podman),
        "docker" => Ok(EngineKind::Docker),
        _ => Err(format!(
            "Unsupported engine '{raw}'. Use 'podman' or 'docker'."
        )),
    }
}

fn is_falsey(value: &str) -> bool {
    matches!(value, "0" | "false" | "no")
}
