use std::collections::HashMap;
use std::env;
use std::io::IsTerminal;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::domain::{Provider, Scope, ServiceInfo};
use crate::infra::engine::Engine;
use crate::infra::process::{spawn_process_group, terminate_process};
use crate::infra::ui::{open_browser, UiServer};
use crate::support::args::{
    extract_subcommand, has_flag, has_project_name, insert_after, is_env_false, take_flag,
};
use crate::support::constants::{BIN_NAME, HISTORY_LIMIT};
use crate::support::logging::{log_worker, LogHub};
use crate::support::services::build_service_info;

struct ProcessHandles {
    compose_proc: Mutex<Option<Child>>,
    log_procs: Mutex<Vec<Child>>,
}

impl ProcessHandles {
    fn new() -> Self {
        Self {
            compose_proc: Mutex::new(None),
            log_procs: Mutex::new(Vec::new()),
        }
    }

    fn stop_log_procs(&self) {
        let mut procs = self.log_procs.lock().unwrap();
        for proc in procs.iter_mut() {
            terminate_process(proc, Duration::from_secs(5));
        }
        procs.clear();
    }

    fn stop_compose_proc(&self) {
        let mut proc = self.compose_proc.lock().unwrap();
        if let Some(child) = proc.as_mut() {
            terminate_process(child, Duration::from_secs(10));
        }
        *proc = None;
    }
}

pub(crate) struct ComposeRunner {
    compose_cmd: Vec<String>,
    compose_file: String,
    compose_file_from_args: bool,
    project_name: String,
    compose_args: Vec<String>,
    provider: Provider,
    engine: Engine,
    stop_event: Arc<AtomicBool>,
    cleanup_enabled: bool,
    cleanup_done: bool,
    signal_handled: Arc<AtomicBool>,
    exit_code: Arc<AtomicI32>,
    handles: Arc<ProcessHandles>,
    project_args: Vec<String>,
    log_hub: Option<Arc<LogHub>>,
    ui_server: Option<UiServer>,
    service_info: Vec<ServiceInfo>,
    log_follow_thread: Option<thread::JoinHandle<i32>>,
    log_threads: Vec<thread::JoinHandle<()>>,
    watchdog_proc: Option<Child>,
}

impl ComposeRunner {
    pub(crate) fn new(
        compose_cmd: Vec<String>,
        provider: Provider,
        engine: Engine,
        compose_file: String,
        project_name: String,
        args: Vec<String>,
    ) -> Self {
        let service_info = build_service_info(&compose_file);
        Self {
            compose_cmd,
            compose_file,
            compose_file_from_args: false,
            project_name,
            compose_args: args,
            provider,
            engine,
            stop_event: Arc::new(AtomicBool::new(false)),
            cleanup_enabled: false,
            cleanup_done: false,
            signal_handled: Arc::new(AtomicBool::new(false)),
            exit_code: Arc::new(AtomicI32::new(0)),
            handles: Arc::new(ProcessHandles::new()),
            project_args: Vec::new(),
            log_hub: None,
            ui_server: None,
            service_info,
            log_follow_thread: None,
            log_threads: Vec::new(),
            watchdog_proc: None,
        }
    }

    pub(crate) fn set_compose_file_from_args(&mut self, from_args: bool) {
        self.compose_file_from_args = from_args;
    }

    pub(crate) fn set_project_args(&mut self, args: Vec<String>) {
        self.project_args = args;
    }

    pub(crate) fn enable_cleanup(&mut self) {
        self.cleanup_enabled = true;
    }

    pub(crate) fn signal_context(&self) -> SignalContext {
        SignalContext {
            stop_event: self.stop_event.clone(),
            signal_handled: self.signal_handled.clone(),
            exit_code: self.exit_code.clone(),
            handles: self.handles.clone(),
        }
    }

    pub(crate) fn signal_exit_code(&self) -> i32 {
        self.exit_code.load(Ordering::SeqCst)
    }

    pub(crate) fn cleanup_once(&mut self) {
        if !self.cleanup_enabled || self.cleanup_done {
            return;
        }
        self.cleanup_done = true;
        self.stop_event.store(true, Ordering::SeqCst);
        self.handles.stop_log_procs();
        self.handles.stop_compose_proc();
        if let Some(handle) = self.log_follow_thread.take() {
            let _ = handle.join();
        }
        for handle in self.log_threads.drain(..) {
            let _ = handle.join();
        }
        if let Some(server) = self.ui_server.as_mut() {
            server.stop();
        }
        self.ui_server = None;
        self.engine.cleanup_project(
            &self.compose_cmd,
            &self.compose_file,
            self.provider,
            &self.project_name,
            &self.project_args,
        );
    }

    pub(crate) fn run(&mut self) -> i32 {
        if self.compose_args.is_empty() {
            eprintln!("Usage: {} <compose-subcommand> [args...]", BIN_NAME);
            return 2;
        }
        let subcommand = extract_subcommand(&self.compose_args)
            .unwrap_or_else(|| self.compose_args[0].clone());
        let mut no_cache_requested = false;
        let mut force_recreate_requested = false;
        if subcommand == "up" {
            let (updated, requested) = take_flag(&self.compose_args, "--no-cache");
            self.compose_args = updated;
            no_cache_requested = requested;
            let (updated, requested) = take_flag(&self.compose_args, "--force-recreate");
            self.compose_args = updated;
            force_recreate_requested = requested;
        }
        if !has_project_name(&self.compose_args) {
            self.project_args = vec!["-p".to_string(), self.project_name.clone()];
        }
        if self.provider == Provider::PodmanCompose && !has_flag(&self.compose_args, &["--in-pod"]) {
            let mut updated = vec!["--in-pod".to_string(), "false".to_string()];
            updated.extend(self.compose_args.iter().cloned());
            self.compose_args = updated;
        }

        if subcommand == "up" {
            if !no_cache_requested
                && !has_flag(&self.compose_args, &["--build"])
                && !has_flag(&self.compose_args, &["--no-build"])
                && !is_env_false("COMPOSE_DEFAULT_BUILD")
            {
                self.compose_args.push("--build".to_string());
            }
            if force_recreate_requested && !has_flag(&self.compose_args, &["--force-recreate"]) {
                self.compose_args = insert_after(&self.compose_args, "up", "--force-recreate");
            }
            if !has_flag(&self.compose_args, &["--remove-orphans"])
                && !is_env_false("COMPOSE_DEFAULT_REMOVE_ORPHANS")
            {
                self.compose_args.push("--remove-orphans".to_string());
            }
        }
        if subcommand == "down" {
            if !has_flag(&self.compose_args, &["--remove-orphans"])
                && !is_env_false("COMPOSE_DEFAULT_REMOVE_ORPHANS")
            {
                self.compose_args.push("--remove-orphans".to_string());
            }
        }

        let no_start_requested = has_flag(&self.compose_args, &["--no-start"]);
        let detach_requested = has_flag(&self.compose_args, &["-d", "--detach"]);
        let ui_enabled = subcommand == "up" && !is_env_false("COMPOSE_LOG_UI");
        if ui_enabled {
            self.start_ui();
        }

        let manual_log_follow = self.engine.manual_log_follow(&subcommand, detach_requested);
        let log_follow_enabled = ui_enabled || manual_log_follow;
        let emit_stdout = self.engine.emit_stdout_for_logs(detach_requested);
        self.cleanup_enabled = log_follow_enabled;
        if self.cleanup_enabled && self.engine.supports_watchdog() {
            self.start_watchdog();
        }

        let auto_start_after_up =
            self.provider == Provider::PodmanCompose && subcommand == "up" && !no_start_requested;
        if auto_start_after_up {
            self.compose_args = insert_after(&self.compose_args, "up", "--no-start");
        }

        let mut follow_in_thread = false;
        if log_follow_enabled && subcommand == "up" {
            if manual_log_follow {
                self.compose_args = insert_after(&self.compose_args, "up", "--detach");
            } else if self.engine.follow_logs_in_thread(&subcommand, detach_requested) {
                follow_in_thread = true;
                self.start_log_follow_thread(false);
            }
        }

        if subcommand == "up" {
            let running_ids = self.engine.collect_container_ids(&self.project_name, Scope::Running);
            let all_ids = self.engine.collect_container_ids(&self.project_name, Scope::All);
            if running_ids.is_empty() && !all_ids.is_empty() {
                self.engine.cleanup_project(
                    &self.compose_cmd,
                    &self.compose_file,
                    self.provider,
                    &self.project_name,
                    &self.project_args,
                );
            }
        }

        if subcommand == "up" && no_cache_requested {
            let exit_code = self.run_compose(&["build".to_string(), "--no-cache".to_string()]);
            if exit_code != 0 {
                return exit_code;
            }
        }

        let exit_code = self.run_compose(&self.compose_args);
        if exit_code != 0 {
            if subcommand == "up" && !auto_start_after_up {
                if self.engine.start_project_containers_with_retries(&self.project_name) {
                    return 0;
                }
                self.engine.cleanup_project(
                    &self.compose_cmd,
                    &self.compose_file,
                    self.provider,
                    &self.project_name,
                    &self.project_args,
                );
            }
            return exit_code;
        }

        if subcommand == "up" && auto_start_after_up {
            if !self.engine.start_project_containers_with_retries(&self.project_name) {
                return 1;
            }
        }

        if log_follow_enabled && subcommand == "up" && !follow_in_thread {
            let follower = self.log_follower();
            return follower.follow_logs(emit_stdout, &mut self.log_threads);
        }

        if subcommand == "down" || subcommand == "stop" {
            self.engine.cleanup_project(
                &self.compose_cmd,
                &self.compose_file,
                self.provider,
                &self.project_name,
                &self.project_args,
            );
        }

        0
    }

    fn run_compose(&self, args: &[String]) -> i32 {
        let mut cmd = Command::new(&self.compose_cmd[0]);
        cmd.args(&self.compose_cmd[1..]);
        if !self.compose_file_from_args {
            cmd.arg("-f").arg(&self.compose_file);
        }
        cmd.args(&self.project_args)
            .args(args)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        let child = match spawn_process_group(&mut cmd) {
            Ok(child) => child,
            Err(err) => {
                eprintln!("[compose] failed to start compose: {}", err);
                return 1;
            }
        };
        {
            let mut proc = self.handles.compose_proc.lock().unwrap();
            *proc = Some(child);
        }
        loop {
            let finished = {
                let mut proc = self.handles.compose_proc.lock().unwrap();
                if let Some(child) = proc.as_mut() {
                    child.try_wait().ok().flatten()
                } else {
                    return 1;
                }
            };
            if let Some(status) = finished {
                return status.code().unwrap_or(1);
            }
            if self.stop_event.load(Ordering::SeqCst) {
                return 1;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    fn start_ui(&mut self) {
        let log_hub = self.log_hub.get_or_insert_with(|| Arc::new(LogHub::new(HISTORY_LIMIT)));
        match UiServer::start(log_hub.clone(), self.service_info.clone(), self.stop_event.clone()) {
            Ok(server) => {
                let port = server.port();
                self.ui_server = Some(server);
                let url = format!("http://127.0.0.1:{}/", port);
                println!("[compose] log UI: {}", url);
                open_browser(&url);
            }
            Err(err) => {
                eprintln!("[compose] log UI failed: {}", err);
            }
        }
    }

    fn start_log_follow_thread(&mut self, emit_stdout: bool) {
        if self.log_follow_thread.is_some() {
            return;
        }
        let follower = self.log_follower();
        let handle = thread::spawn(move || {
            let mut log_threads = Vec::new();
            follower.follow_logs(emit_stdout, &mut log_threads)
        });
        self.log_follow_thread = Some(handle);
    }

    fn log_follower(&self) -> LogFollower {
        LogFollower {
            engine: self.engine.clone(),
            project_name: self.project_name.clone(),
            stop_event: self.stop_event.clone(),
            log_hub: self.log_hub.clone(),
            handles: self.handles.clone(),
        }
    }

    fn start_watchdog(&mut self) {
        if self.watchdog_proc.is_some() {
            return;
        }
        let exe = match env::current_exe() {
            Ok(exe) => exe,
            Err(_) => return,
        };
        let mut cmd = Command::new(exe);
        cmd.arg("--watchdog")
            .arg(format!("{}", std::process::id()))
            .arg(&self.project_name)
            .arg(&self.compose_file)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(conn) = self.engine.connection() {
            cmd.arg(conn);
        }
        if let Ok(child) = spawn_process_group(&mut cmd) {
            self.watchdog_proc = Some(child);
        }
    }
}

struct LogFollower {
    engine: Engine,
    project_name: String,
    stop_event: Arc<AtomicBool>,
    log_hub: Option<Arc<LogHub>>,
    handles: Arc<ProcessHandles>,
}

impl LogFollower {
    fn follow_logs(&self, emit_stdout: bool, log_threads: &mut Vec<thread::JoinHandle<()>>) -> i32 {
        let ids = self.wait_for_container_ids();
        if ids.is_empty() {
            return 1;
        }

        let mut services = Vec::new();
        let mut max_len = 0;
        for cid in &ids {
            let service = self.engine.resolve_service_name(&self.project_name, cid);
            max_len = max_len.max(service.len());
            services.push((cid.clone(), service));
        }

        let mut color_enabled = emit_stdout;
        let mut timestamps_enabled = true;
        if is_env_false("COMPOSE_LOG_COLOR") {
            color_enabled = false;
        }
        if is_env_false("COMPOSE_LOG_TIMESTAMPS") {
            timestamps_enabled = false;
        }
        if emit_stdout && !std::io::stdout().is_terminal() {
            color_enabled = false;
        }

        let colors = [31, 32, 33, 34, 35, 36, 91, 92, 93, 94, 95, 96];
        let mut service_colors = HashMap::new();
        let mut color_index = 0;

        for (cid, service) in services {
            let color_code = *service_colors.entry(service.clone()).or_insert_with(|| {
                let code = colors[color_index % colors.len()];
                color_index += 1;
                code
            });
            let prefix = format!("{:<width$}", service, width = max_len);
            let (color_prefix, color_reset) = if color_enabled {
                (format!("\u{1b}[{}m", color_code), "\u{1b}[0m".to_string())
            } else {
                ("".to_string(), "".to_string())
            };

            let cmd = self.engine.logs_cmd(&cid, timestamps_enabled);
            let mut command = Command::new(&cmd[0]);
            command.args(&cmd[1..]).stdout(Stdio::piped()).stderr(Stdio::piped());
            let mut child = match spawn_process_group(&mut command) {
                Ok(child) => child,
                Err(_) => continue,
            };
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            self.handles.log_procs.lock().unwrap().push(child);

            if let Some(stdout) = stdout {
                let hub = self.log_hub.clone();
                let stop_event = self.stop_event.clone();
                let prefix = prefix.clone();
                let color_prefix = color_prefix.clone();
                let color_reset = color_reset.clone();
                let service_name = service.clone();
                let thread = thread::spawn(move || {
                    log_worker(
                        stdout,
                        hub,
                        stop_event,
                        &service_name,
                        &prefix,
                        &color_prefix,
                        &color_reset,
                        emit_stdout,
                    );
                });
                log_threads.push(thread);
            }
            if let Some(stderr) = stderr {
                let hub = self.log_hub.clone();
                let stop_event = self.stop_event.clone();
                let prefix = prefix.clone();
                let color_prefix = color_prefix.clone();
                let color_reset = color_reset.clone();
                let service_name = service.clone();
                let thread = thread::spawn(move || {
                    log_worker(
                        stderr,
                        hub,
                        stop_event,
                        &service_name,
                        &prefix,
                        &color_prefix,
                        &color_reset,
                        emit_stdout,
                    );
                });
                log_threads.push(thread);
            }
        }

        for handle in log_threads.drain(..) {
            let _ = handle.join();
        }
        0
    }

    fn wait_for_container_ids(&self) -> Vec<String> {
        while !self.stop_event.load(Ordering::SeqCst) {
            let ids = self
                .engine
                .collect_container_ids(&self.project_name, Scope::All);
            if !ids.is_empty() {
                return ids;
            }
            thread::sleep(Duration::from_millis(500));
        }
        Vec::new()
    }
}

pub(crate) struct SignalContext {
    stop_event: Arc<AtomicBool>,
    signal_handled: Arc<AtomicBool>,
    exit_code: Arc<AtomicI32>,
    handles: Arc<ProcessHandles>,
}

impl SignalContext {
    pub(crate) fn handle_signal(&self) {
        if self.signal_handled.swap(true, Ordering::SeqCst) {
            return;
        }
        self.exit_code.store(130, Ordering::SeqCst);
        self.stop_event.store(true, Ordering::SeqCst);
        self.handles.stop_log_procs();
        self.handles.stop_compose_proc();
    }
}
