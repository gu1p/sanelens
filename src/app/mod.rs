mod runner;
mod watchdog;

use signal_hook::consts::signal::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::infra::compose::detect_compose_cmd;
use crate::infra::engine::{CleanupContext, ContainerInfo, Engine};
use crate::infra::ui::{open_browser, UiServer};
use crate::support::args::{
    extract_compose_file_arg, extract_engine_arg, extract_subcommand, extract_traffic_arg,
    first_compose_file, strip_project_name_args,
};
use crate::support::constants::{
    COMPOSE_FILE_LABEL, DERIVED_COMPOSE_LABEL, PROJECT_NAME_LABEL, PROXY_EGRESS_LABEL, PROXY_LABEL,
    RUN_ID_LABEL, SERVICE_LABEL, STARTED_AT_LABEL,
};
use crate::support::logging::LogHub;
use crate::support::run::{new_run_id, project_name_from_run_id, run_started_at};
use crate::support::services::build_service_info;
use crate::support::traffic::TrafficHub;

pub fn run() -> ExitCode {
    match run_inner() {
        Ok(code) => exit_code_from_i32(code),
        Err(err) => {
            eprintln!("{}", err.message);
            ExitCode::from(err.code)
        }
    }
}

struct AppError {
    message: String,
    code: u8,
}

impl AppError {
    fn new(message: impl Into<String>, code: u8) -> Self {
        Self {
            message: message.into(),
            code,
        }
    }
}

enum SessionCommand {
    List,
    Logs { run_id: Option<String> },
    Traffic { run_id: Option<String> },
    Down { run_id: Option<String> },
}

fn run_inner() -> Result<i32, AppError> {
    let args: Vec<String> = env::args().skip(1).collect();
    if handle_version(&args) || handle_watchdog(&args) {
        return Ok(0);
    }

    let (args, engine_preference) =
        extract_engine_arg(&args).map_err(|err| AppError::new(err, 2))?;
    let (args, traffic_override) = extract_traffic_arg(&args);
    let args = strip_project_name_args(&args);
    if let Some(command) = extract_session_command(&args) {
        let selection =
            detect_compose_cmd(engine_preference).map_err(|err| AppError::new(err, 1))?;
        let engine = Engine::new(selection.engine, &selection.compose_cmd);
        let exit_code = match command {
            SessionCommand::List => Ok(run_list(&engine)),
            SessionCommand::Logs { run_id } => match require_run_id("logs", run_id) {
                Ok(run_id) => run_logs(&engine, &run_id),
                Err(err) => Err(err),
            },
            SessionCommand::Traffic { run_id } => match require_run_id("traffic", run_id) {
                Ok(run_id) => run_traffic(&engine, &run_id),
                Err(err) => Err(err),
            },
            SessionCommand::Down { run_id } => match require_run_id("down", run_id) {
                Ok(run_id) => run_down(&engine, &selection.compose_cmd, &run_id),
                Err(err) => Err(err),
            },
        }
        .map_err(|err| AppError::new(err, 2))?;
        return Ok(exit_code);
    }

    let (compose_file, compose_file_from_args) =
        resolve_compose_file(&args).map_err(|err| AppError::new(err, 2))?;
    let run_id = new_run_id();
    let project_name = project_name_from_run_id(&run_id);
    let started_at = run_started_at();
    let selection = detect_compose_cmd(engine_preference).map_err(|err| AppError::new(err, 1))?;
    let engine = Engine::new(selection.engine, &selection.compose_cmd);

    if extract_subcommand(&args).as_deref() == Some("up") {
        let _ = writeln!(std::io::stdout(), "Run ID: {run_id}");
    }

    let mut runner = runner::ComposeRunner::new(runner::ComposeRunnerConfig {
        compose_cmd: selection.compose_cmd,
        engine,
        compose_file,
        run_id,
        project_name,
        run_started_at: started_at,
        args,
    });
    runner.set_compose_file_from_args(compose_file_from_args);
    runner.set_traffic_enabled(traffic_enabled(traffic_override));
    setup_signals(runner.signal_context());

    Ok(run_with_cleanup(&mut runner))
}

fn handle_version(args: &[String]) -> bool {
    if matches!(args, [arg] if arg == "--version" || arg == "-V") {
        print_version();
        return true;
    }
    false
}

fn handle_watchdog(args: &[String]) -> bool {
    if args.first().map(String::as_str) != Some("--watchdog") {
        return false;
    }
    let Some(parent_pid) = args.get(1).and_then(|value| value.parse().ok()) else {
        return true;
    };
    let Some(run_id) = args.get(2) else {
        return true;
    };
    let Some(project_name) = args.get(3) else {
        return true;
    };
    let Some(compose_file) = args.get(4) else {
        return true;
    };
    let connection = args.get(5).cloned();
    watchdog::run_watchdog(parent_pid, run_id, project_name, compose_file, connection);
    true
}

fn resolve_compose_file(args: &[String]) -> Result<(String, bool), String> {
    let compose_file_arg = extract_compose_file_arg(args);
    let compose_file_env = env::var("COMPOSE_FILE").ok();
    let compose_file_from_args = compose_file_arg.is_some() || compose_file_env.is_some();
    let compose_file = if let Some(path) = compose_file_arg {
        path
    } else if let Some(value) = compose_file_env.as_deref() {
        first_compose_file(value).ok_or_else(|| "COMPOSE_FILE is set but empty.".to_string())?
    } else {
        return Err("Compose file is required. Pass -f/--file or set COMPOSE_FILE.".to_string());
    };
    Ok((compose_file, compose_file_from_args))
}

fn traffic_enabled(traffic_override: Option<bool>) -> bool {
    traffic_override.unwrap_or(true)
}

fn require_run_id(command: &str, run_id: Option<String>) -> Result<String, String> {
    run_id.ok_or_else(|| format!("Usage: sanelens {command} <run_id>"))
}

fn extract_session_command(args: &[String]) -> Option<SessionCommand> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--" {
            if let Some(cmd) = iter.next() {
                return parse_session_command(cmd.as_str(), &mut iter);
            }
            return None;
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
        return parse_session_command(arg.as_str(), &mut iter);
    }
    None
}

fn parse_session_command<'a>(
    command: &str,
    iter: &mut impl Iterator<Item = &'a String>,
) -> Option<SessionCommand> {
    match command {
        "list" => Some(SessionCommand::List),
        "logs" => Some(SessionCommand::Logs {
            run_id: iter.next().cloned(),
        }),
        "traffic" => Some(SessionCommand::Traffic {
            run_id: iter.next().cloned(),
        }),
        "down" => Some(SessionCommand::Down {
            run_id: iter.next().cloned(),
        }),
        _ => None,
    }
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

fn run_list(engine: &Engine) -> i32 {
    let mut runs = collect_active_runs(engine);
    if runs.is_empty() {
        let mut stdout = io::stdout();
        let _ = writeln!(stdout, "No active runs.");
        return 0;
    }
    runs.sort_by(|a, b| b.started_at_ts.cmp(&a.started_at_ts));

    let mut run_id_width = "RUN_ID".len();
    let mut started_width = "STARTED".len();
    let mut duration_width = "DURATION".len();
    let mut compose_width = "COMPOSE_FILE".len();

    let now_ts = OffsetDateTime::now_utc().unix_timestamp();
    let rows: Vec<_> = runs
        .into_iter()
        .map(|run| {
            let started = run.started_at_raw.unwrap_or_else(|| "-".to_string());
            let duration = run
                .started_at_ts
                .map_or_else(|| "-".to_string(), |ts| format_duration(now_ts - ts));
            let compose_file = run.compose_file.unwrap_or_else(|| "-".to_string());
            run_id_width = run_id_width.max(run.run_id.len());
            started_width = started_width.max(started.len());
            duration_width = duration_width.max(duration.len());
            compose_width = compose_width.max(compose_file.len());
            (run.run_id, started, duration, compose_file)
        })
        .collect();

    let mut stdout = io::stdout();
    let run_id = "RUN_ID";
    let started = "STARTED";
    let duration = "DURATION";
    let compose = "COMPOSE_FILE";
    let _ = writeln!(
        stdout,
        "{run_id:<run_id_width$}  {started:<started_width$}  {duration:<duration_width$}  {compose:<compose_width$}"
    );
    for (run_id, started, duration, compose_file) in rows {
        let _ = writeln!(
            stdout,
            "{run_id:<run_id_width$}  {started:<started_width$}  {duration:<duration_width$}  {compose_file:<compose_width$}"
        );
    }

    0
}

fn run_logs(engine: &Engine, run_id: &str) -> Result<i32, String> {
    let containers = load_run_containers(engine, run_id, crate::domain::Scope::Running)?;
    let metadata = run_metadata_from_containers(run_id, &containers);
    let services = run_services_from_containers(&containers);
    let project_name = metadata
        .project_name
        .unwrap_or_else(|| project_name_from_run_id(run_id));
    let stop_event = Arc::new(AtomicBool::new(false));
    let signal_handled = Arc::new(AtomicBool::new(false));
    let exit_code = Arc::new(AtomicI32::new(0));
    let handles = Arc::new(runner::ProcessHandles::new());
    setup_signals(runner::SignalContext::new(
        stop_event.clone(),
        signal_handled,
        exit_code.clone(),
        handles.clone(),
    ));

    let log_hub = Arc::new(LogHub::new(crate::support::constants::HISTORY_LIMIT));
    let service_info = metadata
        .compose_file
        .as_deref()
        .map(build_service_info)
        .unwrap_or_default();

    let mut ui_server = None;
    match UiServer::start(log_hub.clone(), service_info, None, stop_event.clone()) {
        Ok(server) => {
            let port = server.port();
            let url = format!("http://127.0.0.1:{port}/");
            let _ = writeln!(std::io::stdout(), "[compose] log UI: {url}");
            open_browser(&url);
            ui_server = Some(server);
        }
        Err(err) => {
            eprintln!("[compose] log UI failed: {err}");
        }
    }

    let follower = runner::LogFollower::new(
        engine.clone(),
        run_id.to_string(),
        project_name,
        stop_event,
        Some(log_hub),
        handles.clone(),
        services.proxy_services,
        services.service_aliases,
    );
    let mut log_threads = Vec::new();
    let exit = follower.follow_logs(true, &mut log_threads);

    handles.stop_log_procs();
    if let Some(server) = ui_server.as_mut() {
        server.stop();
    }
    let signal_exit = exit_code.load(Ordering::SeqCst);
    if signal_exit != 0 {
        return Ok(signal_exit);
    }
    Ok(exit)
}

fn run_traffic(engine: &Engine, run_id: &str) -> Result<i32, String> {
    let containers = load_run_containers(engine, run_id, crate::domain::Scope::Running)?;
    let metadata = run_metadata_from_containers(run_id, &containers);
    let services = run_services_from_containers(&containers);
    let project_name = metadata
        .project_name
        .unwrap_or_else(|| project_name_from_run_id(run_id));
    let tap_dir = metadata
        .derived_compose
        .as_ref()
        .and_then(|path| Path::new(path).parent().map(|dir| dir.join("tap")))
        .filter(|dir| dir.exists());

    let stop_event = Arc::new(AtomicBool::new(false));
    let signal_handled = Arc::new(AtomicBool::new(false));
    let exit_code = Arc::new(AtomicI32::new(0));
    let handles = Arc::new(runner::ProcessHandles::new());
    setup_signals(runner::SignalContext::new(
        stop_event.clone(),
        signal_handled,
        exit_code.clone(),
        handles.clone(),
    ));

    let hub = Arc::new(TrafficHub::new());
    let follower = runner::TrafficFollower::new(
        engine.clone(),
        run_id.to_string(),
        project_name,
        stop_event.clone(),
        handles.clone(),
        hub.clone(),
        services.proxy_services,
        services.service_aliases,
        services.egress_proxy,
        tap_dir,
    );

    let handle = thread::spawn(move || follower.follow());
    let (receiver, snapshot) = hub.register_call_client();
    let mut stdout = io::stdout();
    for call in snapshot {
        let line = serde_json::to_string(&call).unwrap_or_default();
        let _ = writeln!(stdout, "{line}");
        let _ = stdout.flush();
    }
    while !stop_event.load(Ordering::SeqCst) {
        match receiver.recv_timeout(Duration::from_secs(1)) {
            Ok(call) => {
                let line = serde_json::to_string(&call).unwrap_or_default();
                let _ = writeln!(stdout, "{line}");
                let _ = stdout.flush();
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        }
    }

    handles.stop_log_procs();
    let follower_exit = handle.join().map_or(1, |code| code);
    let signal_exit = exit_code.load(Ordering::SeqCst);
    if signal_exit != 0 {
        return Ok(signal_exit);
    }
    Ok(follower_exit)
}

fn run_down(engine: &Engine, compose_cmd: &[String], run_id: &str) -> Result<i32, String> {
    let containers = load_run_containers(engine, run_id, crate::domain::Scope::All)?;
    let metadata = run_metadata_from_containers(run_id, &containers);
    let derived_compose = metadata
        .derived_compose
        .ok_or_else(|| format!("Run {run_id} is missing derived compose metadata."))?;
    let project_name = metadata
        .project_name
        .unwrap_or_else(|| project_name_from_run_id(run_id));

    let project_args: Vec<String> = Vec::new();
    engine.cleanup_project(&CleanupContext {
        compose_cmd,
        compose_file: &derived_compose,
        project_name: &project_name,
        project_args: &project_args,
    });

    if let Some(dir) = Path::new(&derived_compose).parent() {
        if let Err(err) = fs::remove_dir_all(dir) {
            eprintln!("[compose] cleanup failed: {err}");
        }
    }
    Ok(0)
}

fn collect_active_runs(engine: &Engine) -> Vec<RunMetadata> {
    let ids = engine.collect_container_ids_with_label(RUN_ID_LABEL, crate::domain::Scope::Running);
    if ids.is_empty() {
        return Vec::new();
    }
    let containers = engine.inspect_containers(&ids);
    let mut runs: HashMap<String, RunMetadata> = HashMap::new();
    for container in containers {
        let Some(run_id) = container.labels.get(RUN_ID_LABEL) else {
            continue;
        };
        let entry = runs
            .entry(run_id.clone())
            .or_insert_with(|| RunMetadata::new(run_id.clone()));
        entry.apply_labels(&container.labels);
    }
    runs.into_values().collect()
}

fn load_run_containers(
    engine: &Engine,
    run_id: &str,
    scope: crate::domain::Scope,
) -> Result<Vec<ContainerInfo>, String> {
    let ids = engine.collect_run_container_ids(run_id, scope);
    if ids.is_empty() {
        return Err(format!("Run {run_id} not found."));
    }
    Ok(engine.inspect_containers(&ids))
}

fn run_metadata_from_containers(run_id: &str, containers: &[ContainerInfo]) -> RunMetadata {
    let mut metadata = RunMetadata::new(run_id.to_string());
    for container in containers {
        metadata.apply_labels(&container.labels);
    }
    metadata
}

fn run_services_from_containers(containers: &[ContainerInfo]) -> RunServices {
    let mut proxy_services = HashSet::new();
    let mut service_aliases = HashMap::new();
    let mut egress_proxy = None;
    for container in containers {
        let Some(service_name) = container.service.as_ref() else {
            continue;
        };
        if let Some(original) = container.labels.get(SERVICE_LABEL) {
            if original != service_name {
                service_aliases.insert(service_name.clone(), original.clone());
            }
        }
        if container
            .labels
            .get(PROXY_LABEL)
            .is_some_and(|value| label_is_truthy(value))
        {
            proxy_services.insert(service_name.clone());
            if container
                .labels
                .get(PROXY_EGRESS_LABEL)
                .is_some_and(|value| label_is_truthy(value))
            {
                egress_proxy = Some(service_name.clone());
            }
        }
    }
    RunServices {
        proxy_services,
        service_aliases,
        egress_proxy,
    }
}

fn label_is_truthy(value: &str) -> bool {
    matches!(value.to_lowercase().as_str(), "1" | "true" | "yes")
}

fn parse_started_at(value: &str) -> Option<i64> {
    OffsetDateTime::parse(value, &Rfc3339)
        .ok()
        .map(OffsetDateTime::unix_timestamp)
}

fn format_duration(secs: i64) -> String {
    let secs = secs.max(0);
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let minutes = (secs % 3_600) / 60;
    let seconds = secs % 60;

    if days > 0 {
        format!("{days}d{hours}h{minutes}m")
    } else if hours > 0 {
        format!("{hours}h{minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m{seconds}s")
    } else {
        format!("{seconds}s")
    }
}

struct RunMetadata {
    run_id: String,
    compose_file: Option<String>,
    derived_compose: Option<String>,
    project_name: Option<String>,
    started_at_raw: Option<String>,
    started_at_ts: Option<i64>,
}

impl RunMetadata {
    #[allow(clippy::missing_const_for_fn)]
    fn new(run_id: String) -> Self {
        Self {
            run_id,
            compose_file: None,
            derived_compose: None,
            project_name: None,
            started_at_raw: None,
            started_at_ts: None,
        }
    }

    fn apply_labels(&mut self, labels: &HashMap<String, String>) {
        if self.compose_file.is_none() {
            if let Some(value) = labels.get(COMPOSE_FILE_LABEL) {
                self.compose_file = Some(value.clone());
            }
        }
        if self.derived_compose.is_none() {
            if let Some(value) = labels.get(DERIVED_COMPOSE_LABEL) {
                self.derived_compose = Some(value.clone());
            }
        }
        if self.project_name.is_none() {
            if let Some(value) = labels.get(PROJECT_NAME_LABEL) {
                self.project_name = Some(value.clone());
            }
        }
        if self.started_at_raw.is_none() {
            if let Some(value) = labels.get(STARTED_AT_LABEL) {
                self.started_at_raw = Some(value.clone());
                self.started_at_ts = parse_started_at(value);
            }
        }
    }
}

struct RunServices {
    proxy_services: HashSet<String>,
    service_aliases: HashMap<String, String>,
    egress_proxy: Option<String>,
}

fn setup_signals(context: runner::SignalContext) {
    if let Ok(mut signals) = Signals::new([SIGINT, SIGTERM]) {
        thread::spawn(move || {
            for _ in signals.forever() {
                context.handle_signal();
            }
        });
    }
}

fn run_with_cleanup(runner: &mut runner::ComposeRunner) -> i32 {
    let mut exit_code = runner.run();
    runner.cleanup_once();
    let signal_exit = runner.signal_exit_code();
    if signal_exit != 0 {
        exit_code = signal_exit;
    }
    exit_code
}

fn print_version() {
    let version = env!("CARGO_PKG_VERSION");
    let git_sha = option_env!("GIT_SHA").unwrap_or("unknown");
    let build_date = option_env!("BUILD_DATE").unwrap_or("unknown");
    let mut stdout = io::stdout();
    let _ = writeln!(
        stdout,
        "{{\"version\":\"{version}\",\"commit\":\"{git_sha}\",\"build_date\":\"{build_date}\"}}"
    );
}

fn exit_code_from_i32(code: i32) -> ExitCode {
    let code = u8::try_from(code).unwrap_or(1);
    ExitCode::from(code)
}
