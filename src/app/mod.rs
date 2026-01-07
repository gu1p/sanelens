mod runner;
mod watchdog;

use signal_hook::consts::signal::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;
use std::env;
use std::thread;

use crate::infra::compose::{detect_compose_cmd, detect_provider};
use crate::infra::engine::Engine;
use crate::support::args::{
    compose_name_from_file, derive_project_name, extract_compose_file_arg, extract_engine_arg,
    first_compose_file,
};

pub(crate) fn run() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() == 1 && (args[0] == "--version" || args[0] == "-V") {
        print_version();
        return;
    }
    if !args.is_empty() && args[0] == "--watchdog" {
        if args.len() < 4 {
            return;
        }
        let parent_pid: i32 = args[1].parse().unwrap_or(0);
        let project_name = args[2].clone();
        let compose_file = args[3].clone();
        let connection = args.get(4).cloned();
        watchdog::run_watchdog(parent_pid, &project_name, &compose_file, connection);
        return;
    }

    let (args, engine_preference) = extract_engine_arg(&args);
    let compose_file_arg = extract_compose_file_arg(&args);
    let compose_file_env = env::var("COMPOSE_FILE").ok();
    let compose_file = if let Some(path) = compose_file_arg.clone() {
        path
    } else if let Some(value) = compose_file_env.as_deref() {
        match first_compose_file(value) {
            Some(path) => path,
            None => {
                eprintln!("COMPOSE_FILE is set but empty.");
                std::process::exit(2);
            }
        }
    } else {
        eprintln!("Compose file is required. Pass -f/--file or set COMPOSE_FILE.");
        std::process::exit(2);
    };
    let project_name = env::var("COMPOSE_PROJECT_NAME")
        .ok()
        .or_else(|| compose_name_from_file(&compose_file))
        .unwrap_or_else(|| derive_project_name(&compose_file));
    let selection = detect_compose_cmd(engine_preference);
    let provider = detect_provider(&selection.compose_cmd);
    let engine = Engine::new(selection.engine, &selection.compose_cmd);

    let mut runner = runner::ComposeRunner::new(
        selection.compose_cmd,
        provider,
        engine,
        compose_file,
        project_name,
        args.clone(),
    );
    runner.set_compose_file_from_args(compose_file_arg.is_some() || compose_file_env.is_some());
    if let Ok(mut signals) = Signals::new([SIGINT, SIGTERM]) {
        let context = runner.signal_context();
        thread::spawn(move || {
            for _ in signals.forever() {
                context.handle_signal();
            }
        });
    }

    let mut exit_code = runner.run();
    runner.cleanup_once();
    let signal_exit = runner.signal_exit_code();
    if signal_exit != 0 {
        exit_code = signal_exit;
    }
    std::process::exit(exit_code);
}

fn print_version() {
    let version = env!("CARGO_PKG_VERSION");
    let git_sha = option_env!("GIT_SHA").unwrap_or("unknown");
    let build_date = option_env!("BUILD_DATE").unwrap_or("unknown");
    println!(
        "{{\"version\":\"{}\",\"commit\":\"{}\",\"build_date\":\"{}\"}}",
        version, git_sha, build_date
    );
}
