use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, IsTerminal, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::domain::traffic::ObservationSink;
use crate::domain::{Scope, ServiceInfo};
use crate::infra::derive::{derive_compose, DeriveConfig, DerivedCompose};
use crate::infra::engine::{CleanupContext, Engine};
use crate::infra::process::{spawn_process_group, terminate_process};
use crate::infra::resolver::RuntimeResolver;
use crate::infra::traffic::{observation_from_envoy, observation_from_tap, parse_envoy_log_line};
use crate::infra::ui::{open_browser, UiServer};
use crate::support::args::{
    extract_subcommand, has_flag, insert_after, is_env_false, is_env_truthy,
    strip_compose_file_args, take_flag,
};
use crate::support::constants::{BIN_NAME, HISTORY_LIMIT};
use crate::support::logging::{log_worker, LogHub, LogWorkerConfig};
use crate::support::services::build_service_info;
use crate::support::traffic::TrafficHub;

pub struct ProcessHandles {
    compose_proc: Mutex<Option<Child>>,
    log_procs: Mutex<Vec<Child>>,
}

impl ProcessHandles {
    pub const fn new() -> Self {
        Self {
            compose_proc: Mutex::new(None),
            log_procs: Mutex::new(Vec::new()),
        }
    }

    fn log_procs(&self) -> MutexGuard<'_, Vec<Child>> {
        self.log_procs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn compose_proc(&self) -> MutexGuard<'_, Option<Child>> {
        self.compose_proc
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn stop_log_procs(&self) {
        let mut procs = self.log_procs();
        for proc in procs.iter_mut() {
            terminate_process(proc, Duration::from_secs(5));
        }
        procs.clear();
    }

    pub fn stop_compose_proc(&self) {
        let mut proc = self.compose_proc();
        if let Some(child) = proc.as_mut() {
            terminate_process(child, Duration::from_secs(10));
        }
        *proc = None;
    }
}

pub struct ComposeRunnerConfig {
    pub compose_cmd: Vec<String>,
    pub engine: Engine,
    pub compose_file: String,
    pub run_id: String,
    pub project_name: String,
    pub run_started_at: String,
    pub args: Vec<String>,
}

#[allow(clippy::struct_excessive_bools)]
pub struct ComposeRunner {
    compose_cmd: Vec<String>,
    original_compose_file: String,
    compose_file: String,
    compose_file_from_args: bool,
    run_id: String,
    project_name: String,
    run_started_at: String,
    compose_args: Vec<String>,
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
    traffic_enabled: bool,
    traffic_hub: Option<Arc<TrafficHub>>,
    traffic_threads: Vec<thread::JoinHandle<()>>,
    proxy_services: HashSet<String>,
    service_aliases: HashMap<String, String>,
    egress_proxy: Option<String>,
    watchdog_proc: Option<Child>,
    derived_dir: Option<PathBuf>,
    retain_run_dir: bool,
}

#[allow(clippy::struct_excessive_bools)]
struct FollowPlan {
    log_follow_enabled: bool,
    emit_stdout: bool,
    follow_in_thread: bool,
}

struct SubcommandPlan {
    name: String,
    no_cache_requested: bool,
    force_recreate_requested: bool,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy)]
struct LogThreadOptions {
    emit_stdout: bool,
    color_enabled: bool,
    timestamps_enabled: bool,
}

impl ComposeRunner {
    pub fn new(config: ComposeRunnerConfig) -> Self {
        let service_info = build_service_info(&config.compose_file);
        Self {
            compose_cmd: config.compose_cmd,
            original_compose_file: config.compose_file.clone(),
            compose_file: config.compose_file,
            compose_file_from_args: false,
            run_id: config.run_id,
            project_name: config.project_name,
            run_started_at: config.run_started_at,
            compose_args: config.args,
            engine: config.engine,
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
            traffic_enabled: false,
            traffic_hub: None,
            traffic_threads: Vec::new(),
            proxy_services: HashSet::new(),
            service_aliases: HashMap::new(),
            egress_proxy: None,
            watchdog_proc: None,
            derived_dir: None,
            retain_run_dir: false,
        }
    }

    pub const fn set_compose_file_from_args(&mut self, from_args: bool) {
        self.compose_file_from_args = from_args;
    }

    pub const fn set_traffic_enabled(&mut self, enabled: bool) {
        self.traffic_enabled = enabled;
    }

    pub fn set_derived_dir(&mut self, dir: Option<PathBuf>) {
        self.derived_dir = dir;
    }

    pub const fn enable_cleanup(&mut self) {
        self.cleanup_enabled = true;
    }

    fn prepare_derived_compose(&mut self) -> Result<(), String> {
        let envoy_image = if self.traffic_enabled {
            env::var("SANELENS_ENVOY_IMAGE")
                .unwrap_or_else(|_| "envoyproxy/envoy:v1.30-latest".to_string())
        } else {
            "envoyproxy/envoy:v1.30-latest".to_string()
        };
        let mut config = DeriveConfig {
            run_id: self.run_id.clone(),
            run_started_at: self.run_started_at.clone(),
            envoy_image,
            enable_traffic: self.traffic_enabled,
            enable_egress: self.traffic_enabled && is_env_truthy("SANELENS_EGRESS_PROXY"),
            compose_cmd: self.compose_cmd.clone(),
            compose_args: self.compose_args.clone(),
            compose_file_from_args: self.compose_file_from_args,
            disable_pods: self.engine.is_podman(),
        };
        match derive_compose(&self.original_compose_file, &self.project_name, &config) {
            Ok(derived) => {
                self.apply_derived_compose(derived);
                Ok(())
            }
            Err(err) => {
                if !self.traffic_enabled {
                    return Err(err);
                }
                eprintln!("[compose] traffic disabled: {err}");
                self.traffic_enabled = false;
                config.enable_traffic = false;
                config.enable_egress = false;
                let derived =
                    derive_compose(&self.original_compose_file, &self.project_name, &config)?;
                self.apply_derived_compose(derived);
                Ok(())
            }
        }
    }

    fn apply_derived_compose(&mut self, derived: DerivedCompose) {
        self.compose_file = derived.path.to_string_lossy().into_owned();
        self.derived_dir = Some(derived.run_dir);
        self.proxy_services = derived.proxy_services;
        self.service_aliases = derived.app_service_map;
        self.egress_proxy = derived.egress_proxy;
        self.compose_args = strip_compose_file_args(&self.compose_args);
        self.compose_file_from_args = false;
    }

    fn ensure_traffic_hub(&mut self) -> Option<Arc<TrafficHub>> {
        if !self.traffic_enabled {
            return None;
        }
        let hub = self
            .traffic_hub
            .get_or_insert_with(|| Arc::new(TrafficHub::new()));
        Some(hub.clone())
    }

    pub fn signal_context(&self) -> SignalContext {
        SignalContext {
            stop_event: self.stop_event.clone(),
            signal_handled: self.signal_handled.clone(),
            exit_code: self.exit_code.clone(),
            handles: self.handles.clone(),
        }
    }

    pub fn signal_exit_code(&self) -> i32 {
        self.exit_code.load(Ordering::SeqCst)
    }

    pub fn cleanup_once(&mut self) {
        if self.cleanup_done {
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
        for handle in self.traffic_threads.drain(..) {
            let _ = handle.join();
        }
        if let Some(server) = self.ui_server.as_mut() {
            server.stop();
        }
        self.ui_server = None;
        if self.cleanup_enabled {
            self.engine.cleanup_project(&CleanupContext {
                compose_cmd: &self.compose_cmd,
                compose_file: &self.compose_file,
                project_name: &self.project_name,
                project_args: &self.project_args,
            });
        }
        if let Some(dir) = self.derived_dir.take().filter(|_| !self.retain_run_dir) {
            if let Err(err) = fs::remove_dir_all(&dir) {
                eprintln!("[compose] cleanup failed: {err}");
            }
        }
    }

    pub fn run(&mut self) -> i32 {
        let subcommand_plan = match self.prepare_subcommand() {
            Ok(values) => values,
            Err(code) => return code,
        };

        if let Err(err) = self.prepare_derived_compose() {
            eprintln!("[compose] derive failed: {err}");
            return 1;
        }
        self.apply_defaults(&subcommand_plan);
        let follow_plan = self.prepare_follow_plan(&subcommand_plan.name);
        self.maybe_cleanup_before_up(&subcommand_plan.name);

        if let Some(exit_code) = self.run_no_cache_build(&subcommand_plan) {
            return exit_code;
        }

        let exit_code = self.run_compose(&self.compose_args);
        if exit_code != 0 {
            eprintln!("[compose] command failed with exit code {exit_code}");
            return exit_code;
        }

        if let Some(exit_code) = self.maybe_follow_logs(&follow_plan, &subcommand_plan.name) {
            return exit_code;
        }

        self.maybe_cleanup_after(&subcommand_plan.name);
        0
    }

    fn prepare_subcommand(&mut self) -> Result<SubcommandPlan, i32> {
        if self.compose_args.is_empty() {
            eprintln!("Usage: {BIN_NAME} <compose-subcommand> [args...]");
            return Err(2);
        }
        let fallback = self.compose_args.first().cloned().unwrap_or_default();
        let subcommand = extract_subcommand(&self.compose_args).unwrap_or(fallback);
        let (no_cache_requested, force_recreate_requested) = if subcommand == "up" {
            let (updated, no_cache_requested) = take_flag(&self.compose_args, "--no-cache");
            self.compose_args = updated;
            let (updated, force_recreate_requested) =
                take_flag(&self.compose_args, "--force-recreate");
            self.compose_args = updated;
            (no_cache_requested, force_recreate_requested)
        } else {
            (false, false)
        };
        Ok(SubcommandPlan {
            name: subcommand,
            no_cache_requested,
            force_recreate_requested,
        })
    }

    fn apply_defaults(&mut self, plan: &SubcommandPlan) {
        self.project_args.clear();

        if plan.name == "up" {
            if !plan.no_cache_requested
                && !has_flag(&self.compose_args, &["--build"])
                && !has_flag(&self.compose_args, &["--no-build"])
                && is_env_truthy("COMPOSE_DEFAULT_BUILD")
            {
                self.compose_args.push("--build".to_string());
            }
            if plan.force_recreate_requested && !has_flag(&self.compose_args, &["--force-recreate"])
            {
                self.compose_args = insert_after(&self.compose_args, "up", "--force-recreate");
            }
            if self.traffic_enabled
                && !plan.force_recreate_requested
                && !has_flag(&self.compose_args, &["--no-recreate"])
            {
                self.compose_args = insert_after(&self.compose_args, "up", "--force-recreate");
            }
            if !has_flag(&self.compose_args, &["--remove-orphans"])
                && !is_env_false("COMPOSE_DEFAULT_REMOVE_ORPHANS")
            {
                self.compose_args.push("--remove-orphans".to_string());
            }
        }
        if plan.name == "down"
            && !has_flag(&self.compose_args, &["--remove-orphans"])
            && !is_env_false("COMPOSE_DEFAULT_REMOVE_ORPHANS")
        {
            self.compose_args.push("--remove-orphans".to_string());
        }
    }

    fn prepare_follow_plan(&mut self, subcommand: &str) -> FollowPlan {
        let user_no_start_requested = has_flag(&self.compose_args, &["--no-start"]);

        let detach_requested = has_flag(&self.compose_args, &["-d", "--detach"]);
        let ui_enabled = subcommand == "up"
            && !detach_requested
            && (!is_env_false("COMPOSE_LOG_UI") || self.traffic_enabled);
        if ui_enabled {
            self.start_ui();
        }

        let manual_log_follow = self.engine.manual_log_follow(subcommand, detach_requested);
        let mut log_follow_enabled = ui_enabled || manual_log_follow;
        let mut traffic_follow = self.traffic_enabled && !detach_requested;
        let emit_stdout = self.engine.emit_stdout_for_logs(detach_requested);
        if subcommand == "up" && user_no_start_requested {
            if log_follow_enabled || traffic_follow {
                eprintln!("[compose] --no-start requested; skipping log/traffic follow.");
            }
            log_follow_enabled = false;
            traffic_follow = false;
        }
        self.cleanup_enabled =
            (subcommand == "up" && !detach_requested) || log_follow_enabled || traffic_follow;
        self.retain_run_dir = subcommand == "up" && detach_requested;
        if self.cleanup_enabled && self.engine.supports_watchdog() {
            self.start_watchdog();
        }

        let mut follow_in_thread = false;
        if log_follow_enabled && subcommand == "up" {
            if manual_log_follow {
                self.compose_args = insert_after(&self.compose_args, "up", "--detach");
            } else if self
                .engine
                .follow_logs_in_thread(subcommand, detach_requested)
            {
                follow_in_thread = true;
                self.start_log_follow_thread(false);
            }
        }

        if traffic_follow && subcommand == "up" {
            self.start_traffic_follow_thread();
        }

        FollowPlan {
            log_follow_enabled,
            emit_stdout,
            follow_in_thread,
        }
    }

    fn maybe_cleanup_before_up(&self, subcommand: &str) {
        if subcommand != "up" {
            return;
        }
        let running_ids = self
            .engine
            .collect_run_container_ids(&self.run_id, Scope::Running);
        let all_ids = self
            .engine
            .collect_run_container_ids(&self.run_id, Scope::All);
        if running_ids.is_empty() && !all_ids.is_empty() {
            self.engine.cleanup_project(&CleanupContext {
                compose_cmd: &self.compose_cmd,
                compose_file: &self.compose_file,
                project_name: &self.project_name,
                project_args: &self.project_args,
            });
        }
    }

    fn run_no_cache_build(&self, plan: &SubcommandPlan) -> Option<i32> {
        if plan.name != "up" || !plan.no_cache_requested {
            return None;
        }
        let exit_code = self.run_compose(&["build".to_string(), "--no-cache".to_string()]);
        if exit_code != 0 {
            return Some(exit_code);
        }
        None
    }

    fn maybe_follow_logs(&mut self, plan: &FollowPlan, subcommand: &str) -> Option<i32> {
        if plan.log_follow_enabled && subcommand == "up" && !plan.follow_in_thread {
            let follower = self.log_follower();
            return Some(follower.follow_logs(plan.emit_stdout, &mut self.log_threads));
        }
        None
    }

    fn maybe_cleanup_after(&self, subcommand: &str) {
        if subcommand == "down" || subcommand == "stop" {
            self.engine.cleanup_project(&CleanupContext {
                compose_cmd: &self.compose_cmd,
                compose_file: &self.compose_file,
                project_name: &self.project_name,
                project_args: &self.project_args,
            });
        }
    }

    fn run_compose(&self, args: &[String]) -> i32 {
        let Some((compose_bin, compose_args)) = self.compose_cmd.split_first() else {
            eprintln!("[compose] compose command is empty");
            return 1;
        };
        let mut cmd = Command::new(compose_bin);
        cmd.args(compose_args);
        if !self.compose_file_from_args {
            cmd.arg("-f").arg(&self.compose_file);
        }
        cmd.args(&self.project_args)
            .args(args)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        cmd.env_remove("COMPOSE_PROJECT_NAME");
        let child = match spawn_process_group(&mut cmd) {
            Ok(child) => child,
            Err(err) => {
                eprintln!("[compose] failed to start compose: {err}");
                return 1;
            }
        };
        {
            let mut proc = self.handles.compose_proc();
            *proc = Some(child);
        }
        loop {
            let Ok(finished) = self.try_wait_compose() else {
                return 1;
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

    fn try_wait_compose(&self) -> Result<Option<ExitStatus>, ()> {
        let status = self
            .handles
            .compose_proc()
            .as_mut()
            .ok_or(())?
            .try_wait()
            .ok()
            .flatten();
        Ok(status)
    }

    fn start_ui(&mut self) {
        let traffic_hub = self.ensure_traffic_hub();
        let log_hub = self
            .log_hub
            .get_or_insert_with(|| Arc::new(LogHub::new(HISTORY_LIMIT)));
        match UiServer::start(
            log_hub.clone(),
            self.service_info.clone(),
            traffic_hub,
            self.stop_event.clone(),
        ) {
            Ok(server) => {
                let port = server.port();
                self.ui_server = Some(server);
                let url = format!("http://127.0.0.1:{port}/");
                let _ = writeln!(std::io::stdout(), "[compose] log UI: {url}");
                open_browser(&url);
            }
            Err(err) => {
                eprintln!("[compose] log UI failed: {err}");
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

    fn start_traffic_follow_thread(&mut self) {
        if !self.traffic_enabled || !self.traffic_threads.is_empty() {
            return;
        }
        let Some(follower) = self.traffic_follower() else {
            return;
        };
        let handle = thread::spawn(move || {
            let _ = follower.follow();
        });
        self.traffic_threads.push(handle);
    }

    fn log_follower(&self) -> LogFollower {
        LogFollower {
            engine: self.engine.clone(),
            run_id: self.run_id.clone(),
            project_name: self.project_name.clone(),
            stop_event: self.stop_event.clone(),
            log_hub: self.log_hub.clone(),
            handles: self.handles.clone(),
            proxy_services: self.proxy_services.clone(),
            service_aliases: self.service_aliases.clone(),
        }
    }

    fn traffic_follower(&mut self) -> Option<TrafficFollower> {
        if self.proxy_services.is_empty() {
            return None;
        }
        let hub = self.ensure_traffic_hub()?;
        let tap_dir = self
            .derived_dir
            .as_ref()
            .map(|dir| dir.join("tap"))
            .filter(|dir| dir.exists());
        Some(TrafficFollower {
            engine: self.engine.clone(),
            run_id: self.run_id.clone(),
            project_name: self.project_name.clone(),
            stop_event: self.stop_event.clone(),
            handles: self.handles.clone(),
            hub,
            proxy_services: self.proxy_services.clone(),
            service_aliases: self.service_aliases.clone(),
            egress_proxy: self.egress_proxy.clone(),
            tap_dir,
        })
    }

    fn start_watchdog(&mut self) {
        if self.watchdog_proc.is_some() {
            return;
        }
        let Ok(exe) = env::current_exe() else {
            return;
        };
        let mut cmd = Command::new(exe);
        cmd.arg("--watchdog")
            .arg(std::process::id().to_string())
            .arg(&self.run_id)
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

pub struct LogFollower {
    engine: Engine,
    run_id: String,
    project_name: String,
    stop_event: Arc<AtomicBool>,
    log_hub: Option<Arc<LogHub>>,
    handles: Arc<ProcessHandles>,
    proxy_services: HashSet<String>,
    service_aliases: HashMap<String, String>,
}

impl LogFollower {
    #[allow(clippy::too_many_arguments, clippy::missing_const_for_fn)]
    pub fn new(
        engine: Engine,
        run_id: String,
        project_name: String,
        stop_event: Arc<AtomicBool>,
        log_hub: Option<Arc<LogHub>>,
        handles: Arc<ProcessHandles>,
        proxy_services: HashSet<String>,
        service_aliases: HashMap<String, String>,
    ) -> Self {
        Self {
            engine,
            run_id,
            project_name,
            stop_event,
            log_hub,
            handles,
            proxy_services,
            service_aliases,
        }
    }

    pub fn follow_logs(
        &self,
        emit_stdout: bool,
        log_threads: &mut Vec<thread::JoinHandle<()>>,
    ) -> i32 {
        let ids = self.wait_for_container_ids();
        if ids.is_empty() {
            return 1;
        }
        let (services, max_len) = self.collect_services(&ids);
        let (color_enabled, timestamps_enabled) = Self::log_settings(emit_stdout);
        let options = LogThreadOptions {
            emit_stdout,
            color_enabled,
            timestamps_enabled,
        };
        self.spawn_log_threads(services, max_len, options, log_threads);

        for handle in log_threads.drain(..) {
            let _ = handle.join();
        }
        0
    }

    fn collect_services(&self, ids: &[String]) -> (Vec<(String, String)>, usize) {
        let mut services = Vec::new();
        let mut max_len = 0;
        for cid in ids {
            let service = self.engine.resolve_service_name(&self.project_name, cid);
            if self.proxy_services.contains(&service) {
                continue;
            }
            let service = self
                .service_aliases
                .get(&service)
                .cloned()
                .unwrap_or(service);
            max_len = max_len.max(service.len());
            services.push((cid.clone(), service));
        }
        (services, max_len)
    }

    fn log_settings(emit_stdout: bool) -> (bool, bool) {
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
        (color_enabled, timestamps_enabled)
    }

    fn spawn_log_threads(
        &self,
        services: Vec<(String, String)>,
        max_len: usize,
        options: LogThreadOptions,
        log_threads: &mut Vec<thread::JoinHandle<()>>,
    ) {
        let colors = [31, 32, 33, 34, 35, 36, 91, 92, 93, 94, 95, 96];
        let mut service_colors = HashMap::new();
        let mut color_index = 0;
        for (cid, service) in services {
            let color_code = *service_colors.entry(service.clone()).or_insert_with(|| {
                let code = colors
                    .get(color_index % colors.len())
                    .copied()
                    .unwrap_or(37);
                color_index += 1;
                code
            });
            let prefix = format!("{service:<max_len$}");
            let (color_prefix, color_reset) = if options.color_enabled {
                (format!("\u{1b}[{color_code}m"), "\u{1b}[0m".to_string())
            } else {
                (String::new(), String::new())
            };
            let log_cmd = self.engine.logs_cmd(&cid, options.timestamps_enabled);
            let Some((log_bin, log_args)) = log_cmd.split_first() else {
                continue;
            };
            let mut command = Command::new(log_bin);
            command
                .args(log_args)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let Ok(mut child) = spawn_process_group(&mut command) else {
                continue;
            };
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            self.handles.log_procs().push(child);

            if let Some(stdout) = stdout {
                let config = LogWorkerConfig {
                    service: service.clone(),
                    prefix: prefix.clone(),
                    color_prefix: color_prefix.clone(),
                    color_reset: color_reset.clone(),
                    emit_stdout: options.emit_stdout,
                };
                self.spawn_log_worker(stdout, config, log_threads);
            }
            if let Some(stderr) = stderr {
                let config = LogWorkerConfig {
                    service: service.clone(),
                    prefix: prefix.clone(),
                    color_prefix: color_prefix.clone(),
                    color_reset: color_reset.clone(),
                    emit_stdout: options.emit_stdout,
                };
                self.spawn_log_worker(stderr, config, log_threads);
            }
        }
    }

    fn spawn_log_worker<R: Read + Send + 'static>(
        &self,
        reader: R,
        config: LogWorkerConfig,
        log_threads: &mut Vec<thread::JoinHandle<()>>,
    ) {
        let hub = self.log_hub.clone();
        let stop_event = self.stop_event.clone();
        let thread = thread::spawn(move || {
            log_worker(reader, hub.as_ref(), &stop_event, config);
        });
        log_threads.push(thread);
    }

    fn wait_for_container_ids(&self) -> Vec<String> {
        while !self.stop_event.load(Ordering::SeqCst) {
            let ids = self
                .engine
                .collect_run_container_ids(&self.run_id, Scope::Running);
            if !ids.is_empty() {
                return ids;
            }
            thread::sleep(Duration::from_millis(500));
        }
        Vec::new()
    }
}

pub struct TrafficFollower {
    engine: Engine,
    run_id: String,
    project_name: String,
    stop_event: Arc<AtomicBool>,
    handles: Arc<ProcessHandles>,
    hub: Arc<TrafficHub>,
    proxy_services: HashSet<String>,
    service_aliases: HashMap<String, String>,
    egress_proxy: Option<String>,
    tap_dir: Option<PathBuf>,
}

#[derive(Clone)]
struct TrafficWorkerContext {
    hub: Arc<TrafficHub>,
    resolver: Arc<RuntimeResolver>,
    stop_event: Arc<AtomicBool>,
    service_name: String,
    is_egress: bool,
    tap_enabled: bool,
}

#[derive(Clone)]
struct TapWorkerContext {
    hub: Arc<TrafficHub>,
    resolver: Arc<RuntimeResolver>,
    stop_event: Arc<AtomicBool>,
    service_name: String,
    is_egress: bool,
    tap_dir: PathBuf,
}

impl TrafficFollower {
    #[allow(clippy::too_many_arguments, clippy::missing_const_for_fn)]
    pub fn new(
        engine: Engine,
        run_id: String,
        project_name: String,
        stop_event: Arc<AtomicBool>,
        handles: Arc<ProcessHandles>,
        hub: Arc<TrafficHub>,
        proxy_services: HashSet<String>,
        service_aliases: HashMap<String, String>,
        egress_proxy: Option<String>,
        tap_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            engine,
            run_id,
            project_name,
            stop_event,
            handles,
            hub,
            proxy_services,
            service_aliases,
            egress_proxy,
            tap_dir,
        }
    }

    pub fn follow(&self) -> i32 {
        if self.proxy_services.is_empty() {
            return 0;
        }
        let mut workers = Vec::new();
        let mut seen = HashSet::new();
        let mut tap_seen = HashSet::new();

        while !self.stop_event.load(Ordering::SeqCst) {
            let ids = self
                .engine
                .collect_run_proxy_container_ids(&self.run_id, Scope::Running);
            let new_ids: Vec<String> = ids
                .into_iter()
                .filter(|id| seen.insert(id.clone()))
                .collect();
            if !new_ids.is_empty() {
                let resolver = Arc::new(RuntimeResolver::from_engine(
                    &self.engine,
                    &self.run_id,
                    &self.service_aliases,
                ));
                workers.extend(self.spawn_workers(&new_ids, &resolver, &mut tap_seen));
            }
            self.prune_finished_log_procs();
            Self::prune_finished_workers(&mut workers);
            if !self.stop_event.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(250));
            }
        }
        for handle in workers {
            let _ = handle.join();
        }
        0
    }

    fn prune_finished_log_procs(&self) {
        let mut procs = self.handles.log_procs();
        procs.retain_mut(|child| child.try_wait().ok().flatten().is_none());
        drop(procs);
    }

    fn prune_finished_workers(workers: &mut Vec<thread::JoinHandle<()>>) {
        let mut remaining = Vec::with_capacity(workers.len());
        for handle in workers.drain(..) {
            if handle.is_finished() {
                let _ = handle.join();
            } else {
                remaining.push(handle);
            }
        }
        *workers = remaining;
    }

    fn spawn_workers(
        &self,
        ids: &[String],
        resolver: &Arc<RuntimeResolver>,
        tap_seen: &mut HashSet<String>,
    ) -> Vec<thread::JoinHandle<()>> {
        let mut workers = Vec::new();
        for cid in ids {
            let service = self.engine.resolve_service_name(&self.project_name, cid);
            let is_egress = self.egress_proxy.as_deref() == Some(&service);
            let log_cmd = self.engine.logs_cmd(cid, false);
            let Some((log_bin, log_args)) = log_cmd.split_first() else {
                continue;
            };
            let mut command = Command::new(log_bin);
            command
                .args(log_args)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let Ok(mut child) = spawn_process_group(&mut command) else {
                continue;
            };
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            self.handles.log_procs().push(child);
            let context = TrafficWorkerContext {
                hub: self.hub.clone(),
                resolver: resolver.clone(),
                stop_event: self.stop_event.clone(),
                service_name: service.clone(),
                is_egress,
                tap_enabled: self.tap_dir.is_some(),
            };

            if let Some(stdout) = stdout {
                Self::spawn_traffic_worker(stdout, context.clone(), &mut workers);
            }
            if let Some(stderr) = stderr {
                Self::spawn_traffic_worker(stderr, context, &mut workers);
            }

            if let Some(tap_dir) = self.tap_dir_for_service(&service, tap_seen) {
                let tap_context = TapWorkerContext {
                    hub: self.hub.clone(),
                    resolver: resolver.clone(),
                    stop_event: self.stop_event.clone(),
                    service_name: service.clone(),
                    is_egress,
                    tap_dir,
                };
                Self::spawn_tap_worker(tap_context, &mut workers);
            }
        }
        workers
    }

    fn spawn_traffic_worker<R: Read + Send + 'static>(
        reader: R,
        context: TrafficWorkerContext,
        workers: &mut Vec<thread::JoinHandle<()>>,
    ) {
        let thread = thread::spawn(move || {
            traffic_log_worker(reader, context);
        });
        workers.push(thread);
    }

    fn tap_dir_for_service(
        &self,
        service_name: &str,
        tap_seen: &mut HashSet<String>,
    ) -> Option<PathBuf> {
        let tap_root = self.tap_dir.as_ref()?;
        if !tap_seen.insert(service_name.to_string()) {
            return None;
        }
        Some(tap_root.join(service_name))
    }

    fn spawn_tap_worker(context: TapWorkerContext, workers: &mut Vec<thread::JoinHandle<()>>) {
        let thread = thread::spawn(move || {
            tap_file_worker(context);
        });
        workers.push(thread);
    }
}

fn traffic_log_worker<R: Read>(reader: R, context: TrafficWorkerContext) {
    let TrafficWorkerContext {
        hub,
        resolver,
        stop_event,
        service_name,
        is_egress,
        tap_enabled,
    } = context;
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    while !stop_event.load(Ordering::SeqCst) {
        line.clear();
        let Ok(bytes) = reader.read_line(&mut line) else {
            break;
        };
        if bytes == 0 {
            break;
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            continue;
        }
        let Some(log) = parse_envoy_log_line(trimmed) else {
            continue;
        };
        if tap_enabled && (log.method.is_some() || log.path.is_some() || log.authority.is_some()) {
            continue;
        }
        let now_ms = current_time_ms();
        if let Some(obs) =
            observation_from_envoy(log, &service_name, resolver.as_ref(), is_egress, now_ms)
        {
            hub.emit(obs);
        }
    }
}

fn tap_file_worker(context: TapWorkerContext) {
    let TapWorkerContext {
        hub,
        resolver,
        stop_event,
        service_name,
        is_egress,
        tap_dir,
    } = context;
    let _ = fs::create_dir_all(&tap_dir);
    while !stop_event.load(Ordering::SeqCst) {
        let Ok(entries) = fs::read_dir(&tap_dir) else {
            thread::sleep(Duration::from_millis(250));
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Ok(payload) = fs::read_to_string(&path) else {
                continue;
            };
            let now_ms = current_time_ms();
            if let Some(obs) = observation_from_tap(
                &payload,
                &service_name,
                resolver.as_ref(),
                is_egress,
                now_ms,
            ) {
                hub.emit(obs);
                let _ = fs::remove_file(&path);
            }
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn current_time_ms() -> u64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    u64::try_from(millis).unwrap_or(u64::MAX)
}

pub struct SignalContext {
    stop_event: Arc<AtomicBool>,
    signal_handled: Arc<AtomicBool>,
    exit_code: Arc<AtomicI32>,
    handles: Arc<ProcessHandles>,
}

impl SignalContext {
    #[allow(clippy::missing_const_for_fn)]
    pub fn new(
        stop_event: Arc<AtomicBool>,
        signal_handled: Arc<AtomicBool>,
        exit_code: Arc<AtomicI32>,
        handles: Arc<ProcessHandles>,
    ) -> Self {
        Self {
            stop_event,
            signal_handled,
            exit_code,
            handles,
        }
    }

    pub fn handle_signal(&self) {
        if self.signal_handled.swap(true, Ordering::SeqCst) {
            return;
        }
        self.exit_code.store(130, Ordering::SeqCst);
        self.stop_event.store(true, Ordering::SeqCst);
        self.handles.stop_log_procs();
        self.handles.stop_compose_proc();
    }
}
