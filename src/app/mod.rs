mod runner;
mod watchdog;

use signal_hook::consts::signal::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;
use std::env;
use std::io::{self, Write};
use std::process::ExitCode;
use std::thread;

use crate::infra::compose::detect_compose_cmd;
use crate::infra::engine::Engine;
use crate::support::args::{
    compose_name_from_file, derive_project_name, extract_compose_file_arg, extract_engine_arg,
    extract_traffic_arg, first_compose_file,
};

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

fn run_inner() -> Result<i32, AppError> {
    let args: Vec<String> = env::args().skip(1).collect();
    if handle_version(&args) || handle_watchdog(&args) {
        return Ok(0);
    }

    let (args, engine_preference) =
        extract_engine_arg(&args).map_err(|err| AppError::new(err, 2))?;
    let (args, traffic_override) = extract_traffic_arg(&args);
    let (compose_file, compose_file_from_args) =
        resolve_compose_file(&args).map_err(|err| AppError::new(err, 2))?;
    let project_name = resolve_project_name(&compose_file);
    let selection = detect_compose_cmd(engine_preference).map_err(|err| AppError::new(err, 1))?;
    let engine = Engine::new(selection.engine, &selection.compose_cmd);

    let mut runner = runner::ComposeRunner::new(runner::ComposeRunnerConfig {
        compose_cmd: selection.compose_cmd,
        engine,
        compose_file,
        project_name,
        args,
    });
    runner.set_compose_file_from_args(compose_file_from_args);
    runner.set_traffic_enabled(traffic_enabled(traffic_override));
    setup_signals(&runner);

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
    let Some(project_name) = args.get(2) else {
        return true;
    };
    let Some(compose_file) = args.get(3) else {
        return true;
    };
    let connection = args.get(4).cloned();
    watchdog::run_watchdog(parent_pid, project_name, compose_file, connection);
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

fn resolve_project_name(compose_file: &str) -> String {
    env::var("COMPOSE_PROJECT_NAME")
        .ok()
        .or_else(|| compose_name_from_file(compose_file))
        .unwrap_or_else(|| derive_project_name(compose_file))
}

fn traffic_enabled(traffic_override: Option<bool>) -> bool {
    traffic_override.unwrap_or(true)
}

fn setup_signals(runner: &runner::ComposeRunner) {
    if let Ok(mut signals) = Signals::new([SIGINT, SIGTERM]) {
        let context = runner.signal_context();
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
