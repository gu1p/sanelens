use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::domain::{LogEvent, ServiceInfo};
use crate::support::logging::LogHub;

static INDEX_HTML: &str = include_str!("../../assets/compose-ui/index.html");
static APP_JS: &str = include_str!("../../assets/compose-ui/app.js");
static STYLES_CSS: &str = include_str!("../../assets/compose-ui/styles.css");

pub(crate) struct UiServer {
    stop_event: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
    port: u16,
}

impl UiServer {
    pub(crate) fn start(
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

    pub(crate) fn port(&self) -> u16 {
        self.port
    }

    pub(crate) fn stop(&mut self) {
        self.stop_event.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

pub(crate) fn open_browser(url: &str) {
    let _ = webbrowser::open(url);
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

#[derive(serde::Serialize)]
struct ServicesResponse<'a> {
    services: &'a [ServiceInfo],
}
