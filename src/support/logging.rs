use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};
use std::collections::{HashSet, VecDeque};
use std::io::{BufRead, BufReader, Read};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::domain::LogEvent;
use crate::support::constants::CLIENT_QUEUE_SIZE;

struct LogHubState {
    history: VecDeque<LogEvent>,
    clients: Vec<(usize, Sender<LogEvent>)>,
    next_client_id: usize,
}

pub(crate) struct LogHub {
    state: Mutex<LogHubState>,
    seq: AtomicU64,
    history_size: usize,
}

impl LogHub {
    pub(crate) fn new(history_size: usize) -> Self {
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

    pub(crate) fn publish(&self, service: &str, line: &str) {
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

    pub(crate) fn register_client(&self) -> (Receiver<LogEvent>, Vec<LogEvent>) {
        let (sender, receiver) = bounded(CLIENT_QUEUE_SIZE);
        let mut state = self.state.lock().unwrap();
        let id = state.next_client_id;
        state.next_client_id += 1;
        state.clients.push((id, sender));
        let history = state.history.iter().cloned().collect();
        (receiver, history)
    }
}

pub(crate) fn log_worker<R: Read>(
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
