use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::domain::traffic::{TrafficCall, TrafficEdge};
use crate::domain::{LogEvent, ServiceInfo};
use crate::support::logging::LogHub;
use crate::support::traffic::TrafficHub;

static INDEX_HTML: &str = include_str!(env!("SANELENS_INDEX_HTML"));
static APP_JS: &str = include_str!(env!("SANELENS_APP_JS"));
static STYLES_CSS: &str = include_str!(env!("SANELENS_STYLES_CSS"));

pub struct UiServer {
    stop_event: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
    port: u16,
}

impl UiServer {
    pub fn start(
        log_hub: Arc<LogHub>,
        service_info: Vec<ServiceInfo>,
        traffic_hub: Option<Arc<TrafficHub>>,
        stop_event: Arc<AtomicBool>,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let services = Arc::new(service_info);
        let stop_clone = stop_event.clone();
        let handle = thread::spawn(move || {
            run_listener(
                &listener,
                &log_hub,
                &services,
                traffic_hub.as_ref(),
                &stop_clone,
            );
        });
        Ok(Self {
            stop_event,
            handle: Some(handle),
            port,
        })
    }

    pub const fn port(&self) -> u16 {
        self.port
    }

    pub fn stop(&mut self) {
        self.stop_event.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

pub fn open_browser(url: &str) {
    let _ = webbrowser::open(url);
}

enum AcceptOutcome {
    Stream(TcpStream),
    Wait,
    Stop,
}

fn accept_next(listener: &TcpListener) -> AcceptOutcome {
    match listener.accept() {
        Ok((stream, _)) => AcceptOutcome::Stream(stream),
        Err(err) if err.kind() == io::ErrorKind::WouldBlock => AcceptOutcome::Wait,
        Err(_) => AcceptOutcome::Stop,
    }
}

fn run_listener(
    listener: &TcpListener,
    log_hub: &Arc<LogHub>,
    services: &Arc<Vec<ServiceInfo>>,
    traffic_hub: Option<&Arc<TrafficHub>>,
    stop_event: &Arc<AtomicBool>,
) {
    while !stop_event.load(Ordering::SeqCst) {
        match accept_next(listener) {
            AcceptOutcome::Stream(stream) => spawn_connection_handler(
                stream,
                log_hub.clone(),
                services.clone(),
                traffic_hub.cloned(),
                stop_event.clone(),
            ),
            AcceptOutcome::Wait => thread::sleep(Duration::from_millis(100)),
            AcceptOutcome::Stop => return,
        }
    }
}

fn spawn_connection_handler(
    stream: TcpStream,
    log_hub: Arc<LogHub>,
    services: Arc<Vec<ServiceInfo>>,
    traffic_hub: Option<Arc<TrafficHub>>,
    stop_event: Arc<AtomicBool>,
) {
    thread::spawn(move || {
        if let Err(err) = handle_connection(
            stream,
            &log_hub,
            &services,
            traffic_hub.as_ref(),
            &stop_event,
        ) {
            eprintln!("[compose] ui connection error: {err}");
        }
    });
}

struct UiRouteContext<'a> {
    log_hub: &'a Arc<LogHub>,
    service_info: &'a Arc<Vec<ServiceInfo>>,
    traffic_hub: Option<&'a Arc<TrafficHub>>,
    stop_event: &'a Arc<AtomicBool>,
}

fn handle_connection(
    stream: TcpStream,
    log_hub: &Arc<LogHub>,
    service_info: &Arc<Vec<ServiceInfo>>,
    traffic_hub: Option<&Arc<TrafficHub>>,
    stop_event: &Arc<AtomicBool>,
) -> io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let Some(request_line) = read_request_line(&mut reader)? else {
        return Ok(());
    };
    let Some((method, path)) = parse_request_line(&request_line) else {
        return Ok(());
    };
    drain_headers(&mut reader)?;

    if method != "GET" {
        return write_response(stream, 405, "text/plain", b"Method not allowed");
    }

    let context = UiRouteContext {
        log_hub,
        service_info,
        traffic_hub,
        stop_event,
    };
    route_request(path, stream, &context)
}

fn read_request_line(reader: &mut BufReader<TcpStream>) -> io::Result<Option<String>> {
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(None);
    }
    Ok(Some(request_line))
}

fn parse_request_line(line: &str) -> Option<(&str, &str)> {
    let mut parts = line.split_whitespace();
    let method = parts.next()?;
    let path = parts.next().unwrap_or("/");
    let path = path.split('?').next().unwrap_or(path);
    Some((method, path))
}

fn drain_headers(reader: &mut BufReader<TcpStream>) -> io::Result<()> {
    loop {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 || line == "\r\n" {
            break;
        }
    }
    Ok(())
}

fn route_request(path: &str, stream: TcpStream, context: &UiRouteContext<'_>) -> io::Result<()> {
    match path {
        "/" | "/index.html" => write_response(
            stream,
            200,
            "text/html; charset=utf-8",
            INDEX_HTML.as_bytes(),
        ),
        "/app.js" => write_response(
            stream,
            200,
            "application/javascript; charset=utf-8",
            APP_JS.as_bytes(),
        ),
        "/styles.css" => write_response(
            stream,
            200,
            "text/css; charset=utf-8",
            STYLES_CSS.as_bytes(),
        ),
        "/api/services" => write_services_response(stream, context.service_info),
        "/events" => write_event_stream(stream, context.log_hub, context.stop_event),
        "/traffic" => route_traffic_stream(stream, context.traffic_hub, context.stop_event),
        "/traffic/calls" => {
            route_traffic_calls_stream(stream, context.traffic_hub, context.stop_event)
        }
        _ => write_response(stream, 404, "text/plain", b"Not found"),
    }
}

fn write_services_response(
    stream: TcpStream,
    service_info: &Arc<Vec<ServiceInfo>>,
) -> io::Result<()> {
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

fn route_traffic_stream(
    stream: TcpStream,
    traffic_hub: Option<&Arc<TrafficHub>>,
    stop_event: &Arc<AtomicBool>,
) -> io::Result<()> {
    match traffic_hub {
        Some(hub) => write_traffic_stream(stream, hub, stop_event),
        None => write_response(stream, 404, "text/plain", b"Not found"),
    }
}

fn route_traffic_calls_stream(
    stream: TcpStream,
    traffic_hub: Option<&Arc<TrafficHub>>,
    stop_event: &Arc<AtomicBool>,
) -> io::Result<()> {
    match traffic_hub {
        Some(hub) => write_traffic_calls_stream(stream, hub, stop_event),
        None => write_response(stream, 404, "text/plain", b"Not found"),
    }
}

fn write_response(
    stream: TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> io::Result<()> {
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
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "OK",
    };
    let content_len = body.len();
    let mut response = format!(
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: {content_type}\r\nContent-Length: {content_len}\r\n"
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
    log_hub: &Arc<LogHub>,
    stop_event: &Arc<AtomicBool>,
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
    if write_history(&mut stream, &history).is_err() {
        return Ok(());
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

fn write_history(stream: &mut TcpStream, events: &[LogEvent]) -> io::Result<()> {
    let payload = serde_json::to_string(events).unwrap_or_default();
    stream.write_all(format!("event: history\ndata: {payload}\n\n").as_bytes())?;
    stream.flush()?;
    Ok(())
}

fn write_event(stream: &mut TcpStream, event: &LogEvent) -> io::Result<()> {
    let payload = serde_json::to_string(event).unwrap_or_default();
    stream.write_all(format!("data: {payload}\n\n").as_bytes())?;
    stream.flush()?;
    Ok(())
}

fn write_traffic_stream(
    mut stream: TcpStream,
    hub: &Arc<TrafficHub>,
    stop_event: &Arc<AtomicBool>,
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

    let (receiver, snapshot) = hub.register_client();
    if write_traffic_snapshot(&mut stream, &snapshot).is_err() {
        return Ok(());
    }

    while !stop_event.load(Ordering::SeqCst) {
        match receiver.recv_timeout(Duration::from_secs(1)) {
            Ok(event) => {
                if write_traffic_event(&mut stream, &event).is_err() {
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

fn write_traffic_calls_stream(
    mut stream: TcpStream,
    hub: &Arc<TrafficHub>,
    stop_event: &Arc<AtomicBool>,
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

    let (receiver, snapshot) = hub.register_call_client();
    if write_traffic_call_snapshot(&mut stream, &snapshot).is_err() {
        return Ok(());
    }

    while !stop_event.load(Ordering::SeqCst) {
        match receiver.recv_timeout(Duration::from_secs(1)) {
            Ok(event) => {
                if write_traffic_call_event(&mut stream, &event).is_err() {
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

fn write_traffic_snapshot(stream: &mut TcpStream, edges: &[TrafficEdge]) -> io::Result<()> {
    let payload = serde_json::to_string(edges).unwrap_or_default();
    stream.write_all(format!("event: snapshot\ndata: {payload}\n\n").as_bytes())?;
    stream.flush()?;
    Ok(())
}

fn write_traffic_event(stream: &mut TcpStream, edge: &TrafficEdge) -> io::Result<()> {
    let payload = serde_json::to_string(edge).unwrap_or_default();
    stream.write_all(format!("data: {payload}\n\n").as_bytes())?;
    stream.flush()?;
    Ok(())
}

fn write_traffic_call_snapshot(stream: &mut TcpStream, calls: &[TrafficCall]) -> io::Result<()> {
    let payload = serde_json::to_string(calls).unwrap_or_default();
    stream.write_all(format!("event: snapshot\ndata: {payload}\n\n").as_bytes())?;
    stream.flush()?;
    Ok(())
}

fn write_traffic_call_event(stream: &mut TcpStream, call: &TrafficCall) -> io::Result<()> {
    let payload = serde_json::to_string(call).unwrap_or_default();
    stream.write_all(format!("data: {payload}\n\n").as_bytes())?;
    stream.flush()?;
    Ok(())
}

#[derive(serde::Serialize)]
struct ServicesResponse<'a> {
    services: &'a [ServiceInfo],
}
