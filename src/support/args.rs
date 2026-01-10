use std::env;
use std::fs;
use std::path::Path;

use crate::domain::EngineKind;
use crate::support::constants::DEFAULT_PROJECT_NAME;

pub(crate) fn extract_engine_arg(args: &[String]) -> (Vec<String>, Option<EngineKind>) {
    let mut updated = Vec::with_capacity(args.len());
    let mut selected = None;
    let mut iter = args.iter().peekable();
    while let Some(arg) = iter.next() {
        if arg == "--" {
            updated.push(arg.clone());
            updated.extend(iter.cloned());
            break;
        }
        if arg == "--engine" {
            let value = iter.next().map(|value| value.as_str());
            selected = Some(parse_engine_kind(value));
            continue;
        }
        if let Some(value) = arg.strip_prefix("--engine=") {
            selected = Some(parse_engine_kind(Some(value)));
            continue;
        }
        updated.push(arg.clone());
    }
    (updated, selected)
}

pub(crate) fn has_project_name(args: &[String]) -> bool {
    for arg in args {
        if arg == "-p" || arg == "--project-name" {
            return true;
        }
        if arg.starts_with("--project-name=") {
            return true;
        }
    }
    false
}

pub(crate) fn has_flag(args: &[String], names: &[&str]) -> bool {
    for arg in args {
        for name in names {
            if arg == name {
                return true;
            }
            if let Some(value) = arg.strip_prefix(&format!("{}=", name)) {
                let value = value.to_lowercase();
                if value == "0" || value == "false" || value == "no" {
                    break;
                }
                return true;
            }
        }
    }
    false
}

pub(crate) fn extract_subcommand(args: &[String]) -> Option<String> {
    let mut iter = args.iter().peekable();
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

pub(crate) fn take_flag(args: &[String], name: &str) -> (Vec<String>, bool) {
    let mut updated = Vec::with_capacity(args.len());
    let mut enabled = false;
    let prefix = format!("{}=", name);
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

pub(crate) fn extract_compose_file_arg(args: &[String]) -> Option<String> {
    let mut found = None;
    let mut iter = args.iter().peekable();
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
            continue;
        }
    }
    found
}

pub(crate) fn first_compose_file(value: &str) -> Option<String> {
    let separator = if cfg!(windows) { ';' } else { ':' };
    value
        .split(separator)
        .map(str::trim)
        .find(|entry| !entry.is_empty())
        .map(|entry| entry.to_string())
}

pub(crate) fn compose_name_from_file(compose_file: &str) -> Option<String> {
    let contents = fs::read_to_string(compose_file).ok()?;
    let doc: serde_yaml::Value = serde_yaml::from_str(&contents).ok()?;
    doc.get("name")?.as_str().map(|name| name.to_string())
}

pub(crate) fn derive_project_name(compose_file: &str) -> String {
    let base = Path::new(compose_file)
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string())
        .or_else(|| {
            env::current_dir().ok().and_then(|dir| {
                dir.file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| name.to_string())
            })
        })
        .unwrap_or_else(|| DEFAULT_PROJECT_NAME.to_string());
    sanitize_project_name(&base)
}

fn sanitize_project_name(name: &str) -> String {
    let mut output = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            output.push(ch.to_ascii_lowercase());
        } else {
            output.push('_');
        }
    }
    let trimmed = output.trim_matches(|c| c == '_' || c == '-');
    if trimmed.is_empty() {
        DEFAULT_PROJECT_NAME.to_string()
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn insert_after(args: &[String], token: &str, new_arg: &str) -> Vec<String> {
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

pub(crate) fn is_env_false(name: &str) -> bool {
    match env::var(name) {
        Ok(value) => matches!(value.to_lowercase().as_str(), "0" | "false" | "no"),
        Err(_) => false,
    }
}

fn parse_engine_kind(value: Option<&str>) -> EngineKind {
    let raw = match value {
        Some(value) => value,
        None => {
            eprintln!("--engine requires a value of 'podman' or 'docker'.");
            std::process::exit(2);
        }
    };
    match raw.to_lowercase().as_str() {
        "podman" => EngineKind::Podman,
        "docker" => EngineKind::Docker,
        _ => {
            eprintln!("Unsupported engine '{}'. Use 'podman' or 'docker'.", raw);
            std::process::exit(2);
        }
    }
}
