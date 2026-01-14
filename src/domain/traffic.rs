use serde::Serialize;
use std::collections::BTreeMap;
use std::net::IpAddr;

#[derive(Clone, Debug, Hash, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EntityId {
    Workload {
        name: String,
        instance: Option<String>,
    },
    External {
        ip: IpAddr,
        dns_name: Option<String>,
    },
    #[allow(dead_code)]
    Host {
        name: String,
    },
    Unknown,
}

#[derive(Clone, Debug, Hash, Eq, PartialEq, Serialize)]
pub struct Socket {
    pub ip: IpAddr,
    pub port: u16,
}

#[derive(Clone, Debug, Hash, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Transport {
    Tcp,
    #[allow(dead_code)]
    Udp,
    #[allow(dead_code)]
    Other {
        code: u8,
    },
}

#[derive(Clone, Debug, Hash, Eq, PartialEq, Serialize)]
pub struct FlowKey {
    pub src: Socket,
    pub dst: Socket,
    pub transport: Transport,
}

#[derive(Clone, Debug, Serialize)]
pub struct FlowMetrics {
    pub bytes_in: Option<u64>,
    pub bytes_out: Option<u64>,
    pub packets: Option<u64>,
    pub duration_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Peer {
    pub src: Option<EntityId>,
    pub dst: Option<EntityId>,
    pub raw: Option<FlowKey>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ObservationAttrs {
    pub visibility: Visibility,
    pub confidence: Confidence,
    pub tags: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    L4Flow,
    L7Envelope,
    L7Semantics,
}

impl Visibility {
    pub const fn merge(current: &Self, next: &Self) -> Self {
        match (current, next) {
            (Self::L7Semantics, _) | (_, Self::L7Semantics) => Self::L7Semantics,
            (Self::L7Envelope, _) | (_, Self::L7Envelope) => Self::L7Envelope,
            _ => Self::L4Flow,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Exact,
    Likely,
    #[allow(dead_code)]
    Uncertain,
}

#[allow(clippy::struct_field_names)]
#[derive(Clone, Debug, Default, Serialize)]
pub struct Correlation {
    pub request_id: Option<String>,
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct HttpObservation {
    pub at_ms: u64,
    pub peer: Peer,
    pub method: Option<String>,
    pub path: Option<String>,
    pub status: Option<u16>,
    pub duration_ms: Option<u64>,
    pub bytes_in: Option<u64>,
    pub bytes_out: Option<u64>,
    pub request_headers: BTreeMap<String, String>,
    pub response_headers: BTreeMap<String, String>,
    pub request_body: Option<String>,
    pub response_body: Option<String>,
    pub correlation: Correlation,
    pub attrs: ObservationAttrs,
}

#[derive(Clone, Debug, Serialize)]
pub struct FlowObservation {
    pub at_ms: u64,
    pub flow: FlowKey,
    pub metrics: FlowMetrics,
    pub peer: Peer,
    pub attrs: ObservationAttrs,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Observation {
    Flow(FlowObservation),
    Http(HttpObservation),
}

#[derive(Clone, Debug, Serialize)]
pub struct TrafficCall {
    pub seq: u64,
    pub at_ms: u64,
    pub peer: Peer,
    pub method: Option<String>,
    pub path: Option<String>,
    pub status: Option<u16>,
    pub duration_ms: Option<u64>,
    pub bytes_in: Option<u64>,
    pub bytes_out: Option<u64>,
    pub request_headers: BTreeMap<String, String>,
    pub response_headers: BTreeMap<String, String>,
    pub request_body: Option<String>,
    pub response_body: Option<String>,
    pub correlation: Correlation,
    pub attrs: ObservationAttrs,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Default, Serialize)]
pub struct Capabilities {
    pub l4_flows: bool,
    pub http: bool,
    pub grpc: bool,
    pub can_see_bodies: bool,
    pub can_correlate: bool,
}

pub trait ObservationSink: Send + Sync {
    fn emit(&self, obs: Observation);
}

pub trait Resolver: Send + Sync {
    fn resolve_entity(&self, socket: &Socket) -> Option<EntityId>;
}

#[derive(Clone, Debug, Hash, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EdgeKey {
    Flow {
        from: EntityId,
        to: EntityId,
        transport: Transport,
        port: u16,
    },
    Http {
        from: EntityId,
        to: EntityId,
        method: String,
        route: String,
    },
    #[allow(dead_code)]
    Grpc {
        from: EntityId,
        to: EntityId,
        service: String,
        method: String,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct EdgeStats {
    pub count: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub errors: u64,
    pub p50_ms: Option<u64>,
    pub p95_ms: Option<u64>,
    pub visibility: Visibility,
}

#[derive(Clone, Debug, Serialize)]
pub struct TrafficEdge {
    pub key: EdgeKey,
    pub stats: EdgeStats,
    pub last_seen_ms: u64,
}
