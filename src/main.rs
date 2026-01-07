use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};
use serde::Serialize;
use signal_hook::consts::signal::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;
use std::collections::{HashMap, HashSet, VecDeque};
use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, IsTerminal, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const HISTORY_LIMIT: usize = 2000;
const CLIENT_QUEUE_SIZE: usize = 1000;
const DEFAULT_PROJECT_NAME: &str = "compose";
const BIN_NAME: &str = "composeui";

static INDEX_HTML: &str = include_str!("../assets/compose-ui/index.html");
static APP_JS: &str = include_str!("../assets/compose-ui/app.js");
static STYLES_CSS: &str = include_str!("../assets/compose-ui/styles.css");

#[derive(Clone, Serialize)]
struct ServiceInfo {
    name: String,
    endpoints: Vec<String>,
    endpoint: Option<String>,
    exposed: bool,
}

#[derive(Clone, Serialize)]
struct LogEvent {
    seq: u64,
    service: String,
    line: String,
}

struct LogHubState {
    history: VecDeque<LogEvent>,
    clients: Vec<(usize, Sender<LogEvent>)>,
    next_client_id: usize,
}

struct LogHub {
    state: Mutex<LogHubState>,
    seq: AtomicU64,
    history_size: usize,
}

impl LogHub {
    fn new(history_size: usize) -> Self {
        Self {
            state: Mutex::new(LogHubState {
                history: VecDeque::with_capacity(history_size),
                clients: Vec::new(),
                next_client_id: 1,
            }),
            seq: AtomicU64::new(0),
            history_size,
        }
    }

    fn publish(&self, service: &str, line: &str) {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let event = LogEvent {
            seq,
            service: if service.is_empty() {
                "unknown".to_string()
            } else {
                service.to_string()
            },
            line: line.to_string(),
        };
        let clients = {
            let mut state = self.state.lock().unwrap();
            state.history.push_back(event.clone());
            while state.history.len() > self.history_size {
                state.history.pop_front();
            }
            state.clients.clone()
        };
        let mut disconnected = HashSet::new();
        for (id, sender) in clients {
            match sender.try_send(event.clone()) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {}
                Err(TrySendError::Disconnected(_)) => {
                    disconnected.insert(id);
                }
            }
        }
        if !disconnected.is_empty() {
            let mut state = self.state.lock().unwrap();
            state.clients.retain(|(id, _)| !disconnected.contains(id));
        }
    }

    fn register_client(&self) -> (Receiver<LogEvent>, Vec<LogEvent>) {
        let (sender, receiver) = bounded(CLIENT_QUEUE_SIZE);
        let mut state = self.state.lock().unwrap();
        let id = state.next_client_id;
        state.next_client_id += 1;
        state.clients.push((id, sender));
        let history = state.history.iter().cloned().collect();
        (receiver, history)
    }
}

struct UiServer {
    stop_event: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
    port: u16,
}

impl UiServer {
    fn start(
        log_hub: Arc<LogHub>,
        service_info: Vec<ServiceInfo>,
        stop_event: Arc<AtomicBool>,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let services = Arc::new(service_info);
        let stop_clone = stop_event.clone();
        let handle = thread::spawn(move || {
            while !stop_clone.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let log_hub = log_hub.clone();
                        let services = services.clone();
                        let stop_event = stop_clone.clone();
                        thread::spawn(move || {
                            if let Err(err) = handle_connection(stream, log_hub, services, stop_event) {
                                eprintln!("[compose] ui connection error: {}", err);
                            }
                        });
                    }
                    Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(100));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            stop_event,
            handle: Some(handle),
            port,
        })
    }

    fn port(&self) -> u16 {
        self.port
    }

    fn stop(&mut self) {
        self.stop_event.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn handle_connection(
    stream: TcpStream,
    log_hub: Arc<LogHub>,
    service_info: Arc<Vec<ServiceInfo>>,
    stop_event: Arc<AtomicBool>,
) -> io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("/");

    loop {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 || line == "\r\n" {
            break;
        }
    }

    if method != "GET" {
        return write_response(stream, 405, "text/plain", b"Method not allowed");
    }

    match path {
        "/" | "/index.html" => {
            write_response(stream, 200, "text/html; charset=utf-8", INDEX_HTML.as_bytes())
        }
        "/app.js" => {
            write_response(stream, 200, "application/javascript; charset=utf-8", APP_JS.as_bytes())
        }
        "/styles.css" => {
            write_response(stream, 200, "text/css; charset=utf-8", STYLES_CSS.as_bytes())
        }
        "/api/services" => {
            let payload = serde_json::to_vec(&ServicesResponse {
                services: service_info.as_slice(),
            })
            .unwrap_or_default();
            write_response_with_headers(
                stream,
                200,
                "application/json",
                &payload,
                &["Cache-Control: no-store"],
            )
        }
        "/events" => write_event_stream(stream, log_hub, stop_event),
        _ => write_response(stream, 404, "text/plain", b"Not found"),
    }
}

fn write_response(stream: TcpStream, status: u16, content_type: &str, body: &[u8]) -> io::Result<()> {
    write_response_with_headers(stream, status, content_type, body, &[])
}

fn write_response_with_headers(
    mut stream: TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    headers: &[&str],
) -> io::Result<()> {
    let status_text = match status {
        200 => "OK",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "OK",
    };
    let mut response = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n",
        status,
        status_text,
        content_type,
        body.len()
    );
    for header in headers {
        response.push_str(header);
        response.push_str("\r\n");
    }
    response.push_str("\r\n");
    stream.write_all(response.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()?;
    Ok(())
}

fn write_event_stream(
    mut stream: TcpStream,
    log_hub: Arc<LogHub>,
    stop_event: Arc<AtomicBool>,
) -> io::Result<()> {
    let headers = [
        "HTTP/1.1 200 OK",
        "Content-Type: text/event-stream",
        "Cache-Control: no-cache",
        "Connection: keep-alive",
        "\r\n",
    ]
    .join("\r\n");
    stream.write_all(headers.as_bytes())?;
    stream.flush()?;

    let (receiver, history) = log_hub.register_client();
    for event in history {
        if write_event(&mut stream, &event).is_err() {
            return Ok(());
        }
    }

    while !stop_event.load(Ordering::SeqCst) {
        match receiver.recv_timeout(Duration::from_secs(1)) {
            Ok(event) => {
                if write_event(&mut stream, &event).is_err() {
                    break;
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                if stream.write_all(b": ping\n\n").is_err() {
                    break;
                }
                let _ = stream.flush();
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}

fn write_event(stream: &mut TcpStream, event: &LogEvent) -> io::Result<()> {
    let payload = serde_json::to_string(event).unwrap_or_default();
    stream.write_all(format!("data: {}\n\n", payload).as_bytes())?;
    stream.flush()?;
    Ok(())
}

#[derive(Serialize)]
struct ServicesResponse<'a> {
    services: &'a [ServiceInfo],
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Runtime {
    Podman,
    Docker,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Provider {
    PodmanCompose,
    Other,
}

#[derive(Clone, Copy)]
enum Scope {
    Running,
    All,
}

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

struct ComposeRunner {
    compose_cmd: Vec<String>,
    compose_file: String,
    compose_file_from_args: bool,
    project_name: String,
    compose_args: Vec<String>,
    runtime: Runtime,
    provider: Provider,
    conn: Option<String>,
    podman_cmd: Vec<String>,
    docker_cmd: Vec<String>,
    stop_event: Arc<AtomicBool>,
    cleanup_enabled: bool,
    cleanup_done: bool,
    signal_handled: Arc<AtomicBool>,
    exit_code: Arc<AtomicI32>,
    handles: Arc<ProcessHandles>,
    project_args: Vec<String>,
    log_hub: Option<Arc<LogHub>>,
    ui_server: Option<UiServer>,
    ui_enabled: bool,
    service_info: Vec<ServiceInfo>,
    log_follow_thread: Option<thread::JoinHandle<i32>>,
    log_threads: Vec<thread::JoinHandle<()>>,
    watchdog_proc: Option<Child>,
}

impl ComposeRunner {
    fn new(compose_cmd: Vec<String>, compose_file: String, project_name: String, args: Vec<String>) -> Self {
        let runtime = if compose_cmd
            .first()
            .map(|cmd| cmd.contains("podman"))
            .unwrap_or(false)
        {
            Runtime::Podman
        } else {
            Runtime::Docker
        };
        let provider = detect_provider(&compose_cmd);
        let conn = env::var("PODMAN_CONNECTION").ok().or_else(|| extract_connection(&compose_cmd));
        let mut podman_cmd = vec!["podman".to_string()];
        if let Some(ref conn) = conn {
            podman_cmd.push("--connection".to_string());
            podman_cmd.push(conn.to_string());
        }
        let docker_cmd = vec!["docker".to_string()];
        let service_info = build_service_info(&compose_file);
        Self {
            compose_cmd,
            compose_file,
            compose_file_from_args: false,
            project_name,
            compose_args: args,
            runtime,
            provider,
            conn,
            podman_cmd,
            docker_cmd,
            stop_event: Arc::new(AtomicBool::new(false)),
            cleanup_enabled: false,
            cleanup_done: false,
            signal_handled: Arc::new(AtomicBool::new(false)),
            exit_code: Arc::new(AtomicI32::new(0)),
            handles: Arc::new(ProcessHandles::new()),
            project_args: Vec::new(),
            log_hub: None,
            ui_server: None,
            ui_enabled: false,
            service_info,
            log_follow_thread: None,
            log_threads: Vec::new(),
            watchdog_proc: None,
        }
    }

    fn signal_context(&self) -> SignalContext {
        SignalContext {
            stop_event: self.stop_event.clone(),
            signal_handled: self.signal_handled.clone(),
            exit_code: self.exit_code.clone(),
            handles: self.handles.clone(),
        }
    }

    fn cleanup_once(&mut self) {
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
        if self.runtime == Runtime::Podman {
            self.cleanup_project();
        }
    }

    fn run(&mut self) -> i32 {
        if self.compose_args.is_empty() {
            eprintln!("Usage: {} <compose-subcommand> [args...]", BIN_NAME);
            return 2;
        }
        let subcommand = self.compose_args[0].clone();
        if !has_project_name(&self.compose_args) {
            self.project_args = vec!["-p".to_string(), self.project_name.clone()];
        }
        if self.provider == Provider::PodmanCompose && !has_flag(&self.compose_args, &["--in-pod"]) {
            let mut updated = vec!["--in-pod".to_string(), "false".to_string()];
            updated.extend(self.compose_args.iter().cloned());
            self.compose_args = updated;
        }

        if subcommand == "up" {
            if !has_flag(&self.compose_args, &["--build"])
                && !has_flag(&self.compose_args, &["--no-build"])
                && !is_env_false("COMPOSE_DEFAULT_BUILD")
            {
                self.compose_args.push("--build".to_string());
            }
            if !has_flag(&self.compose_args, &["--remove-orphans"])
                && !is_env_false("COMPOSE_DEFAULT_REMOVE_ORPHANS")
            {
                self.compose_args.push("--remove-orphans".to_string());
            }
        }

        let no_start_requested = has_flag(&self.compose_args, &["--no-start"]);
        let detach_requested = has_flag(&self.compose_args, &["-d", "--detach"]);
        let ui_enabled = subcommand == "up"
            && !is_env_false("COMPOSE_LOG_UI");
        self.ui_enabled = ui_enabled;
        if ui_enabled {
            self.start_ui();
        }

        let manual_log_follow = self.runtime == Runtime::Podman && subcommand == "up" && !detach_requested;
        let log_follow_enabled = ui_enabled || manual_log_follow;
        let emit_stdout = self.runtime == Runtime::Podman && !detach_requested;
        self.cleanup_enabled = log_follow_enabled;
        if self.cleanup_enabled && self.runtime == Runtime::Podman {
            self.start_watchdog();
        }

        let auto_start_after_up =
            self.provider == Provider::PodmanCompose && subcommand == "up" && !no_start_requested;
        if auto_start_after_up {
            self.compose_args = insert_after(&self.compose_args, "up", "--no-start");
        }

        let mut follow_in_thread = false;
        if log_follow_enabled && subcommand == "up" {
            if self.runtime == Runtime::Podman && !detach_requested {
                self.compose_args = insert_after(&self.compose_args, "up", "--detach");
            } else if self.runtime == Runtime::Docker && !detach_requested {
                follow_in_thread = true;
                self.start_log_follow_thread(false);
            }
        }

        if self.runtime == Runtime::Podman && subcommand == "up" {
            let running_ids = self.collect_container_ids(Scope::Running);
            let all_ids = self.collect_container_ids(Scope::All);
            if running_ids.is_empty() && !all_ids.is_empty() {
                self.cleanup_project();
            }
        }

        let exit_code = self.run_compose(&self.compose_args);
        if exit_code != 0 {
            if self.runtime == Runtime::Podman && subcommand == "up" && !auto_start_after_up {
                if self.start_project_containers_with_retries() {
                    return 0;
                }
                self.cleanup_project();
            }
            return exit_code;
        }

        if self.runtime == Runtime::Podman && subcommand == "up" && auto_start_after_up {
            if !self.start_project_containers_with_retries() {
                return 1;
            }
        }

        if log_follow_enabled && subcommand == "up" && !follow_in_thread {
            let follower = self.log_follower();
            return follower.follow_logs(emit_stdout, &mut self.log_threads);
        }

        if self.runtime == Runtime::Podman && (subcommand == "down" || subcommand == "stop") {
            self.cleanup_project();
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
            runtime: self.runtime,
            project_name: self.project_name.clone(),
            podman_cmd: self.podman_cmd.clone(),
            docker_cmd: self.docker_cmd.clone(),
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
        if let Some(ref conn) = self.conn {
            cmd.arg(conn);
        }
        if let Ok(child) = spawn_process_group(&mut cmd) {
            self.watchdog_proc = Some(child);
        }
    }

    fn collect_container_ids(&self, scope: Scope) -> Vec<String> {
        match self.runtime {
            Runtime::Podman => collect_podman_container_ids(&self.podman_cmd, &self.project_name, scope),
            Runtime::Docker => collect_docker_container_ids(&self.docker_cmd, &self.project_name, scope),
        }
    }

    fn cleanup_project(&self) {
        if self.runtime != Runtime::Podman {
            return;
        }
        self.compose_down();
        remove_project_pods(&self.podman_cmd, &self.project_name);
        let mut ids = collect_podman_container_ids(&self.podman_cmd, &self.project_name, Scope::All);
        ids.extend(collect_podman_container_ids_by_name(&self.podman_cmd, &self.project_name));
        ids.sort();
        ids.dedup();
        if !ids.is_empty() {
            let mut cmd = self.podman_cmd.clone();
            cmd.push("rm".to_string());
            cmd.push("-f".to_string());
            cmd.extend(ids);
            let _ = run_output(&cmd);
        }
    }

    fn compose_down(&self) {
        if self.runtime != Runtime::Podman {
            return;
        }
        let mut cmd = self.compose_cmd.clone();
        if self.provider == Provider::PodmanCompose && !has_flag(&cmd, &["--in-pod"]) {
            cmd.push("--in-pod".to_string());
            cmd.push("false".to_string());
        }
        cmd.push("-f".to_string());
        cmd.push(self.compose_file.clone());
        cmd.extend(self.project_args.clone());
        cmd.push("down".to_string());
        cmd.push("--remove-orphans".to_string());
        let _ = run_output(&cmd);
    }

    fn start_project_containers_with_retries(&self) -> bool {
        let attempts = 3;
        for i in 0..attempts {
            if self.start_project_containers() {
                return true;
            }
            thread::sleep(Duration::from_secs(2 * (i + 1) as u64));
        }
        false
    }

    fn start_project_containers(&self) -> bool {
        if self.runtime != Runtime::Podman {
            return false;
        }
        let ids = collect_podman_container_ids(&self.podman_cmd, &self.project_name, Scope::All);
        if ids.is_empty() {
            return false;
        }
        let mut cmd = self.podman_cmd.clone();
        cmd.push("start".to_string());
        cmd.extend(ids);
        run_output(&cmd).map(|output| output.status.success()).unwrap_or(false)
    }
}

struct LogFollower {
    runtime: Runtime,
    project_name: String,
    podman_cmd: Vec<String>,
    docker_cmd: Vec<String>,
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
            let service = self.resolve_service_name(cid);
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

            let mut cmd = if self.runtime == Runtime::Podman {
                self.podman_cmd.clone()
            } else {
                self.docker_cmd.clone()
            };
            cmd.push("logs".to_string());
            cmd.push("--follow".to_string());
            if timestamps_enabled {
                cmd.push("--timestamps".to_string());
            }
            cmd.push(cid.clone());
            let mut command = Command::new(&cmd[0]);
            command
                .args(&cmd[1..])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
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
            let ids = match self.runtime {
                Runtime::Podman => collect_podman_container_ids(&self.podman_cmd, &self.project_name, Scope::All),
                Runtime::Docker => collect_docker_container_ids(&self.docker_cmd, &self.project_name, Scope::All),
            };
            if !ids.is_empty() {
                return ids;
            }
            thread::sleep(Duration::from_millis(500));
        }
        Vec::new()
    }

    fn resolve_service_name(&self, cid: &str) -> String {
        match self.runtime {
            Runtime::Podman => resolve_service_name_podman(&self.podman_cmd, &self.project_name, cid),
            Runtime::Docker => resolve_service_name_docker(&self.docker_cmd, &self.project_name, cid),
        }
    }
}

struct SignalContext {
    stop_event: Arc<AtomicBool>,
    signal_handled: Arc<AtomicBool>,
    exit_code: Arc<AtomicI32>,
    handles: Arc<ProcessHandles>,
}

impl SignalContext {
    fn handle_signal(&self) {
        if self.signal_handled.swap(true, Ordering::SeqCst) {
            return;
        }
        self.exit_code.store(130, Ordering::SeqCst);
        self.stop_event.store(true, Ordering::SeqCst);
        self.handles.stop_log_procs();
        self.handles.stop_compose_proc();
    }
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

fn main() {
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
        run_watchdog(parent_pid, &project_name, &compose_file, connection);
        return;
    }

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
    let compose_cmd = detect_compose_cmd();

    let mut runner = ComposeRunner::new(compose_cmd, compose_file, project_name, args.clone());
    runner.compose_file_from_args = compose_file_arg.is_some() || compose_file_env.is_some();
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
    let signal_exit = runner.exit_code.load(Ordering::SeqCst);
    if signal_exit != 0 {
        exit_code = signal_exit;
    }
    std::process::exit(exit_code);
}

fn run_watchdog(parent_pid: i32, project_name: &str, compose_file: &str, connection: Option<String>) {
    if parent_pid <= 0 {
        return;
    }
    while pid_alive(parent_pid) {
        thread::sleep(Duration::from_secs(1));
    }
    let compose_cmd = if command_exists("podman") {
        vec!["podman".to_string(), "compose".to_string()]
    } else {
        detect_compose_cmd()
    };
    let mut runner = ComposeRunner::new(
        compose_cmd,
        compose_file.to_string(),
        project_name.to_string(),
        Vec::new(),
    );
    if let Some(conn) = connection {
        runner.conn = Some(conn.clone());
        runner.podman_cmd = vec!["podman".to_string(), "--connection".to_string(), conn];
    }
    runner.project_args = vec!["-p".to_string(), project_name.to_string()];
    runner.cleanup_enabled = true;
    runner.cleanup_once();
}

fn detect_compose_cmd() -> Vec<String> {
    if let Ok(env_cmd) = env::var("COMPOSE_CMD") {
        match shell_words::split(&env_cmd) {
            Ok(cmd) if !cmd.is_empty() => return cmd,
            _ => {
                eprintln!("COMPOSE_CMD is set but empty or invalid.");
                std::process::exit(1);
            }
        }
    }

    if command_exists("podman") {
        if run_status(&["podman".to_string(), "compose".to_string(), "version".to_string()]) {
            let mut cmd = vec!["podman".to_string()];
            if let Ok(conn) = env::var("PODMAN_CONNECTION") {
                cmd.push("--connection".to_string());
                cmd.push(conn);
            }
            cmd.push("compose".to_string());
            return cmd;
        }
    }

    if command_exists("docker")
        && run_status(&["docker".to_string(), "compose".to_string(), "version".to_string()])
    {
        return vec!["docker".to_string(), "compose".to_string()];
    }

    if command_exists("podman-compose") {
        return vec!["podman-compose".to_string()];
    }

    if command_exists("docker-compose") {
        return vec!["docker-compose".to_string()];
    }

    eprintln!("No compose tool found in PATH.");
    std::process::exit(1);
}

fn detect_provider(compose_cmd: &[String]) -> Provider {
    if compose_cmd.is_empty() {
        return Provider::Other;
    }
    if compose_cmd[0] == "podman-compose" {
        return Provider::PodmanCompose;
    }
    if compose_cmd.len() >= 2 && compose_cmd[0] == "podman" && compose_cmd[1] == "compose" {
        let mut cmd = compose_cmd.to_vec();
        cmd.push("version".to_string());
        if let Ok(output) = run_output(&cmd) {
            let mut combined = String::new();
            combined.push_str(&String::from_utf8_lossy(&output.stdout));
            combined.push_str(&String::from_utf8_lossy(&output.stderr));
            if combined.to_lowercase().contains("podman-compose") {
                return Provider::PodmanCompose;
            }
        }
    }
    Provider::Other
}

fn extract_connection(compose_cmd: &[String]) -> Option<String> {
    if compose_cmd.is_empty() || compose_cmd[0] != "podman" {
        return None;
    }
    for idx in 0..compose_cmd.len() {
        let arg = &compose_cmd[idx];
        if arg == "--connection" && idx + 1 < compose_cmd.len() {
            return Some(compose_cmd[idx + 1].clone());
        }
        if let Some(rest) = arg.strip_prefix("--connection=") {
            return Some(rest.to_string());
        }
    }
    None
}

fn has_project_name(args: &[String]) -> bool {
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

fn has_flag(args: &[String], names: &[&str]) -> bool {
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

fn extract_compose_file_arg(args: &[String]) -> Option<String> {
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

fn first_compose_file(value: &str) -> Option<String> {
    let separator = if cfg!(windows) { ';' } else { ':' };
    value
        .split(separator)
        .map(str::trim)
        .find(|entry| !entry.is_empty())
        .map(|entry| entry.to_string())
}

fn compose_name_from_file(compose_file: &str) -> Option<String> {
    let contents = fs::read_to_string(compose_file).ok()?;
    let doc: serde_yaml::Value = serde_yaml::from_str(&contents).ok()?;
    doc.get("name")?.as_str().map(|name| name.to_string())
}

fn derive_project_name(compose_file: &str) -> String {
    let base = Path::new(compose_file)
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string())
        .or_else(|| {
            env::current_dir()
                .ok()
                .and_then(|dir| dir.file_name().and_then(|name| name.to_str()).map(|name| name.to_string()))
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

fn insert_after(args: &[String], token: &str, new_arg: &str) -> Vec<String> {
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

fn is_env_false(name: &str) -> bool {
    match env::var(name) {
        Ok(value) => matches!(value.to_lowercase().as_str(), "0" | "false" | "no"),
        Err(_) => false,
    }
}

fn command_exists(cmd: &str) -> bool {
    if cmd.contains(std::path::MAIN_SEPARATOR) {
        return Path::new(cmd).is_file();
    }
    if let Ok(path) = env::var("PATH") {
        for entry in env::split_paths(&path) {
            let candidate = entry.join(cmd);
            if candidate.is_file() {
                return true;
            }
        }
    }
    false
}

fn run_status(cmd: &[String]) -> bool {
    run_output(cmd).map(|output| output.status.success()).unwrap_or(false)
}

fn run_output(cmd: &[String]) -> io::Result<Output> {
    let mut command = Command::new(&cmd[0]);
    command.args(&cmd[1..]).stdout(Stdio::piped()).stderr(Stdio::piped());
    command.output()
}

fn collect_podman_container_ids(podman_cmd: &[String], project_name: &str, scope: Scope) -> Vec<String> {
    let mut ids = HashSet::new();
    let base = build_podman_ps_cmd(podman_cmd, scope);
    let labels = [
        format!("label=io.podman.compose.project={}", project_name),
        format!("label=com.docker.compose.project={}", project_name),
    ];
    for label in &labels {
        let mut cmd = base.clone();
        cmd.push("--filter".to_string());
        cmd.push(label.to_string());
        cmd.push("-q".to_string());
        if let Ok(output) = run_output(&cmd) {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if !line.trim().is_empty() {
                    ids.insert(line.trim().to_string());
                }
            }
        }
    }
    let mut list: Vec<String> = ids.into_iter().collect();
    list.sort();
    list
}

fn build_podman_ps_cmd(podman_cmd: &[String], scope: Scope) -> Vec<String> {
    let mut cmd = podman_cmd.to_vec();
    cmd.push("ps".to_string());
    if matches!(scope, Scope::All) {
        cmd.push("-a".to_string());
    }
    cmd
}

fn collect_docker_container_ids(docker_cmd: &[String], project_name: &str, scope: Scope) -> Vec<String> {
    let mut cmd = docker_cmd.to_vec();
    cmd.push("ps".to_string());
    if matches!(scope, Scope::All) {
        cmd.push("-a".to_string());
    }
    cmd.push("--filter".to_string());
    cmd.push(format!("label=com.docker.compose.project={}", project_name));
    cmd.push("-q".to_string());
    let mut ids = Vec::new();
    if let Ok(output) = run_output(&cmd) {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if !line.trim().is_empty() {
                ids.push(line.trim().to_string());
            }
        }
    }
    ids.sort();
    ids.dedup();
    ids
}

fn collect_podman_container_ids_by_name(podman_cmd: &[String], project_name: &str) -> Vec<String> {
    let mut cmd = podman_cmd.to_vec();
    cmd.push("ps".to_string());
    cmd.push("-a".to_string());
    cmd.push("--format".to_string());
    cmd.push("{{.ID}} {{.Names}}".to_string());
    let mut ids = HashSet::new();
    if let Ok(output) = run_output(&cmd) {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let mut parts = line.splitn(2, ' ');
            let id = parts.next().unwrap_or("");
            let name = parts.next().unwrap_or("");
            if name.starts_with(&format!("{}-", project_name))
                || name.starts_with(&format!("{}_", project_name))
            {
                if !id.trim().is_empty() {
                    ids.insert(id.trim().to_string());
                }
            }
        }
    }
    ids.into_iter().collect()
}

fn remove_project_pods(podman_cmd: &[String], project_name: &str) {
    let mut cmd = podman_cmd.to_vec();
    cmd.push("pod".to_string());
    cmd.push("ps".to_string());
    cmd.push("-a".to_string());
    cmd.push("--format".to_string());
    cmd.push("{{.Id}} {{.Name}}".to_string());
    let mut pod_ids = Vec::new();
    if let Ok(output) = run_output(&cmd) {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let mut parts = line.splitn(2, ' ');
            let id = parts.next().unwrap_or("");
            let name = parts.next().unwrap_or("");
            if name == format!("pod_{}", project_name) || name.starts_with(&format!("{}-", project_name)) {
                if !id.trim().is_empty() {
                    pod_ids.push(id.trim().to_string());
                }
            }
        }
    }
    if pod_ids.is_empty() {
        return;
    }
    let mut rm_cmd = podman_cmd.to_vec();
    rm_cmd.push("pod".to_string());
    rm_cmd.push("rm".to_string());
    rm_cmd.push("-f".to_string());
    rm_cmd.extend(pod_ids);
    let _ = run_output(&rm_cmd);
}

fn resolve_service_name_podman(podman_cmd: &[String], project_name: &str, cid: &str) -> String {
    let label_keys = [
        "io.podman.compose.service",
        "com.docker.compose.service",
    ];
    for label in &label_keys {
        let mut cmd = podman_cmd.to_vec();
        cmd.push("inspect".to_string());
        cmd.push("--format".to_string());
        cmd.push(format!("{{{{ index .Config.Labels \"{}\" }}}}", label));
        cmd.push(cid.to_string());
        if let Ok(output) = run_output(&cmd) {
            let candidate = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !candidate.is_empty() && candidate != "<no value>" {
                return candidate;
            }
        }
    }
    let mut cmd = podman_cmd.to_vec();
    cmd.push("inspect".to_string());
    cmd.push("--format".to_string());
    cmd.push("{{ .Name }}".to_string());
    cmd.push(cid.to_string());
    if let Ok(output) = run_output(&cmd) {
        let mut name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if name.starts_with('/') {
            name.remove(0);
        }
        name = strip_service_suffix(&name, project_name);
        if !name.is_empty() {
            return name;
        }
    }
    cid.to_string()
}

fn resolve_service_name_docker(docker_cmd: &[String], project_name: &str, cid: &str) -> String {
    let label_keys = ["com.docker.compose.service"];
    for label in &label_keys {
        let mut cmd = docker_cmd.to_vec();
        cmd.push("inspect".to_string());
        cmd.push("--format".to_string());
        cmd.push(format!("{{{{ index .Config.Labels \"{}\" }}}}", label));
        cmd.push(cid.to_string());
        if let Ok(output) = run_output(&cmd) {
            let candidate = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !candidate.is_empty() && candidate != "<no value>" {
                return candidate;
            }
        }
    }
    let mut cmd = docker_cmd.to_vec();
    cmd.push("inspect".to_string());
    cmd.push("--format".to_string());
    cmd.push("{{ .Name }}".to_string());
    cmd.push(cid.to_string());
    if let Ok(output) = run_output(&cmd) {
        let mut name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if name.starts_with('/') {
            name.remove(0);
        }
        name = strip_service_suffix(&name, project_name);
        if !name.is_empty() {
            return name;
        }
    }
    cid.to_string()
}

fn strip_service_suffix(name: &str, project_name: &str) -> String {
    let mut result = name.to_string();
    let prefix = format!("{}_", project_name);
    if result.starts_with(&prefix) {
        result = result[prefix.len()..].to_string();
    }
    let prefix = format!("{}-", project_name);
    if result.starts_with(&prefix) {
        result = result[prefix.len()..].to_string();
    }
    if result.ends_with("_1") {
        let len = result.len();
        result.truncate(len - 2);
    }
    if result.ends_with("-1") {
        let len = result.len();
        result.truncate(len - 2);
    }
    result
}

fn log_worker<R: Read>(
    reader: R,
    log_hub: Option<Arc<LogHub>>,
    stop_event: Arc<AtomicBool>,
    service: &str,
    prefix: &str,
    color_prefix: &str,
    color_reset: &str,
    emit_stdout: bool,
) {
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    loop {
        if stop_event.load(Ordering::SeqCst) {
            break;
        }
        line.clear();
        let bytes = match reader.read_line(&mut line) {
            Ok(bytes) => bytes,
            Err(_) => break,
        };
        if bytes == 0 {
            break;
        }
        if line.ends_with('\n') {
            line.pop();
            if line.ends_with('\r') {
                line.pop();
            }
        }
        if let Some(hub) = log_hub.as_ref() {
            hub.publish(service, &line);
        }
        if emit_stdout {
            println!("{}{}{} | {}", color_prefix, prefix, color_reset, line);
        }
    }
}

fn spawn_process_group(cmd: &mut Command) -> io::Result<Child> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    cmd.spawn()
}

fn terminate_process(child: &mut Child, timeout: Duration) {
    if child.try_wait().ok().flatten().is_some() {
        return;
    }
    #[cfg(unix)]
    {
        let pid = child.id() as i32;
        unsafe {
            libc::killpg(pid, libc::SIGTERM);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }
    if wait_child_timeout(child, timeout) {
        return;
    }
    #[cfg(unix)]
    unsafe {
        libc::killpg(child.id() as i32, libc::SIGKILL);
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }
    let _ = wait_child_timeout(child, Duration::from_secs(1));
}

fn wait_child_timeout(child: &mut Child, timeout: Duration) -> bool {
    let start = Instant::now();
    loop {
        if child.try_wait().ok().flatten().is_some() {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn pid_alive(pid: i32) -> bool {
    #[cfg(unix)]
    unsafe {
        if libc::kill(pid, 0) == 0 {
            return true;
        }
        let err = io::Error::last_os_error();
        return err.raw_os_error() == Some(libc::EPERM);
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    {
        if command_exists("open") {
            let _ = Command::new("open").arg(url).spawn();
        }
    }
    #[cfg(target_os = "linux")]
    {
        if command_exists("xdg-open") {
            let _ = Command::new("xdg-open").arg(url).spawn();
        }
    }
    #[cfg(target_os = "windows")]
    {
        let _ = Command::new("cmd").args(["/C", "start", "", url]).spawn();
    }
}

fn build_service_info(compose_file: &str) -> Vec<ServiceInfo> {
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

fn parse_compose_services_and_ports(compose_file: &str) -> (Vec<String>, HashMap<String, Vec<String>>) {
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
            if let Some(ports_val) = service_map.get(&serde_yaml::Value::String("ports".to_string())) {
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
                                if let Some(value) = map.get(&serde_yaml::Value::String("published".to_string())) {
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
