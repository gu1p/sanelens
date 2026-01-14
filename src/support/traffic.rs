use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, MutexGuard};

use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};

use crate::domain::traffic::{
    EdgeKey, EdgeStats, EntityId, FlowObservation, HttpObservation, Observation, ObservationSink,
    TrafficCall, TrafficEdge, Visibility,
};
use crate::support::constants::{TRAFFIC_CALL_HISTORY_LIMIT, TRAFFIC_CLIENT_QUEUE_SIZE};

const LATENCY_SAMPLE_LIMIT: usize = 256;

struct EdgeState {
    stats: EdgeStats,
    latencies: VecDeque<u64>,
    last_seen_ms: u64,
}

struct TrafficHubState {
    edges: HashMap<EdgeKey, EdgeState>,
    clients: Vec<(usize, Sender<TrafficEdge>)>,
    next_client_id: usize,
    calls: VecDeque<TrafficCall>,
    call_clients: Vec<(usize, Sender<TrafficCall>)>,
    next_call_client_id: usize,
    next_call_seq: u64,
}

pub struct TrafficHub {
    state: Mutex<TrafficHubState>,
}

impl TrafficHub {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(TrafficHubState {
                edges: HashMap::new(),
                clients: Vec::new(),
                next_client_id: 1,
                calls: VecDeque::with_capacity(TRAFFIC_CALL_HISTORY_LIMIT),
                call_clients: Vec::new(),
                next_call_client_id: 1,
                next_call_seq: 1,
            }),
        }
    }

    pub fn register_client(&self) -> (Receiver<TrafficEdge>, Vec<TrafficEdge>) {
        let (sender, receiver) = bounded(TRAFFIC_CLIENT_QUEUE_SIZE);
        let mut state = self.state();
        let id = state.next_client_id;
        state.next_client_id += 1;
        state.clients.push((id, sender));
        let snapshot = state
            .edges
            .iter()
            .map(|(key, edge)| TrafficEdge {
                key: key.clone(),
                stats: edge.stats.clone(),
                last_seen_ms: edge.last_seen_ms,
            })
            .collect();
        drop(state);
        (receiver, snapshot)
    }

    pub fn register_call_client(&self) -> (Receiver<TrafficCall>, Vec<TrafficCall>) {
        let (sender, receiver) = bounded(TRAFFIC_CLIENT_QUEUE_SIZE);
        let mut state = self.state();
        let id = state.next_call_client_id;
        state.next_call_client_id += 1;
        state.call_clients.push((id, sender));
        let snapshot = state.calls.iter().cloned().collect();
        drop(state);
        (receiver, snapshot)
    }

    fn publish(&self, edge: &TrafficEdge) {
        let clients = {
            let mut state = self.state();
            if let Some(existing) = state.edges.get_mut(&edge.key) {
                existing.stats = edge.stats.clone();
                existing.last_seen_ms = edge.last_seen_ms;
            } else {
                state.edges.insert(
                    edge.key.clone(),
                    EdgeState {
                        stats: edge.stats.clone(),
                        latencies: VecDeque::new(),
                        last_seen_ms: edge.last_seen_ms,
                    },
                );
            }
            state.clients.clone()
        };
        let mut disconnected = Vec::new();
        for (id, sender) in clients {
            match sender.try_send(edge.clone()) {
                Ok(()) | Err(TrySendError::Full(_)) => {}
                Err(TrySendError::Disconnected(_)) => {
                    disconnected.push(id);
                }
            }
        }
        if !disconnected.is_empty() {
            let mut state = self.state();
            state.clients.retain(|(id, _)| !disconnected.contains(id));
        }
    }

    fn state(&self) -> MutexGuard<'_, TrafficHubState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn emit_http(&self, http: &HttpObservation) {
        let from = http.peer.src.clone().unwrap_or(EntityId::Unknown);
        let to = http.peer.dst.clone().unwrap_or(EntityId::Unknown);
        let method = http.method.as_deref().unwrap_or("UNKNOWN").to_uppercase();
        let route = http.path.clone().unwrap_or_else(|| "/".to_string());
        let key = EdgeKey::Http {
            from,
            to,
            method,
            route,
        };
        let mut state = self.state();
        let edge = state.edges.entry(key.clone()).or_insert_with(|| EdgeState {
            stats: EdgeStats {
                count: 0,
                bytes_in: 0,
                bytes_out: 0,
                errors: 0,
                p50_ms: None,
                p95_ms: None,
                visibility: http.attrs.visibility.clone(),
            },
            latencies: VecDeque::new(),
            last_seen_ms: http.at_ms,
        });
        edge.stats.count += 1;
        edge.stats.bytes_in += http.bytes_in.unwrap_or(0);
        edge.stats.bytes_out += http.bytes_out.unwrap_or(0);
        if let Some(status) = http.status {
            if status >= 400 {
                edge.stats.errors += 1;
            }
        }
        edge.stats.visibility = Visibility::merge(&edge.stats.visibility, &http.attrs.visibility);
        edge.last_seen_ms = http.at_ms;
        if let Some(duration) = http.duration_ms {
            edge.latencies.push_back(duration);
            while edge.latencies.len() > LATENCY_SAMPLE_LIMIT {
                edge.latencies.pop_front();
            }
            update_latency_stats(&mut edge.stats, &edge.latencies);
        }
        let snapshot = TrafficEdge {
            key,
            stats: edge.stats.clone(),
            last_seen_ms: edge.last_seen_ms,
        };
        drop(state);
        self.publish(&snapshot);
        self.publish_call(http);
    }

    fn emit_flow(&self, flow: FlowObservation) {
        let from = flow.peer.src.unwrap_or(EntityId::Unknown);
        let to = flow.peer.dst.unwrap_or(EntityId::Unknown);
        let port = flow.flow.dst.port;
        let key = EdgeKey::Flow {
            from,
            to,
            transport: flow.flow.transport.clone(),
            port,
        };
        let mut state = self.state();
        let edge = state.edges.entry(key.clone()).or_insert_with(|| EdgeState {
            stats: EdgeStats {
                count: 0,
                bytes_in: 0,
                bytes_out: 0,
                errors: 0,
                p50_ms: None,
                p95_ms: None,
                visibility: flow.attrs.visibility.clone(),
            },
            latencies: VecDeque::new(),
            last_seen_ms: flow.at_ms,
        });
        edge.stats.count += 1;
        edge.stats.bytes_in += flow.metrics.bytes_in.unwrap_or(0);
        edge.stats.bytes_out += flow.metrics.bytes_out.unwrap_or(0);
        edge.stats.visibility = Visibility::merge(&edge.stats.visibility, &flow.attrs.visibility);
        edge.last_seen_ms = flow.at_ms;
        let snapshot = TrafficEdge {
            key,
            stats: edge.stats.clone(),
            last_seen_ms: edge.last_seen_ms,
        };
        drop(state);
        self.publish(&snapshot);
    }

    fn publish_call(&self, http: &HttpObservation) {
        let (call, clients) = {
            let mut state = self.state();
            let seq = state.next_call_seq;
            state.next_call_seq += 1;
            let call = TrafficCall {
                seq,
                at_ms: http.at_ms,
                peer: http.peer.clone(),
                method: http.method.clone(),
                path: http.path.clone(),
                status: http.status,
                duration_ms: http.duration_ms,
                bytes_in: http.bytes_in,
                bytes_out: http.bytes_out,
                request_headers: http.request_headers.clone(),
                response_headers: http.response_headers.clone(),
                request_body: http.request_body.clone(),
                response_body: http.response_body.clone(),
                correlation: http.correlation.clone(),
                attrs: http.attrs.clone(),
            };
            state.calls.push_back(call.clone());
            while state.calls.len() > TRAFFIC_CALL_HISTORY_LIMIT {
                state.calls.pop_front();
            }
            (call, state.call_clients.clone())
        };

        let mut disconnected = Vec::new();
        for (id, sender) in clients {
            match sender.try_send(call.clone()) {
                Ok(()) | Err(TrySendError::Full(_)) => {}
                Err(TrySendError::Disconnected(_)) => {
                    disconnected.push(id);
                }
            }
        }
        if !disconnected.is_empty() {
            let mut state = self.state();
            state
                .call_clients
                .retain(|(id, _)| !disconnected.contains(id));
        }
    }
}

impl ObservationSink for TrafficHub {
    fn emit(&self, obs: Observation) {
        match obs {
            Observation::Http(http) => self.emit_http(&http),
            Observation::Flow(flow) => self.emit_flow(flow),
        }
    }
}

fn update_latency_stats(stats: &mut EdgeStats, samples: &VecDeque<u64>) {
    if samples.is_empty() {
        stats.p50_ms = None;
        stats.p95_ms = None;
        return;
    }
    let mut sorted: Vec<u64> = samples.iter().copied().collect();
    sorted.sort_unstable();
    stats.p50_ms = Some(percentile(&sorted, 50));
    stats.p95_ms = Some(percentile(&sorted, 95));
}

fn percentile(sorted: &[u64], pct: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = (sorted.len() - 1) * pct / 100;
    sorted.get(idx).copied().unwrap_or(0)
}
