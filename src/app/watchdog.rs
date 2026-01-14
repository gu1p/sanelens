use std::thread;
use std::time::Duration;

use crate::app::runner::{ComposeRunner, ComposeRunnerConfig};
use crate::domain::EngineKind;
use crate::infra::compose::detect_compose_cmd;
use crate::infra::engine::Engine;
use crate::infra::process::{command_exists, pid_alive};
use crate::support::run::run_started_at;

pub fn run_watchdog(
    parent_pid: i32,
    run_id: &str,
    project_name: &str,
    compose_file: &str,
    connection: Option<String>,
) {
    if parent_pid <= 0 {
        return;
    }
    while pid_alive(parent_pid) {
        thread::sleep(Duration::from_secs(1));
    }
    let (compose_cmd, engine_kind) = if command_exists("podman") {
        (
            vec!["podman".to_string(), "compose".to_string()],
            EngineKind::Podman,
        )
    } else {
        let selection = match detect_compose_cmd(None) {
            Ok(selection) => selection,
            Err(err) => {
                eprintln!("{err}");
                return;
            }
        };
        (selection.compose_cmd, selection.engine)
    };
    let engine = Engine::new(engine_kind, &compose_cmd).with_connection(connection);
    let mut runner = ComposeRunner::new(ComposeRunnerConfig {
        compose_cmd,
        engine,
        compose_file: compose_file.to_string(),
        run_id: run_id.to_string(),
        project_name: project_name.to_string(),
        run_started_at: run_started_at(),
        args: Vec::new(),
    });
    let derived_dir = std::path::Path::new(compose_file)
        .parent()
        .map(std::path::Path::to_path_buf);
    runner.set_derived_dir(derived_dir);
    runner.enable_cleanup();
    runner.cleanup_once();
}
