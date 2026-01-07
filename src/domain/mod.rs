use serde::Serialize;

#[derive(Clone, Serialize)]
pub(crate) struct ServiceInfo {
    pub(crate) name: String,
    pub(crate) endpoints: Vec<String>,
    pub(crate) endpoint: Option<String>,
    pub(crate) exposed: bool,
}

#[derive(Clone, Serialize)]
pub(crate) struct LogEvent {
    pub(crate) seq: u64,
    pub(crate) service: String,
    pub(crate) line: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Provider {
    PodmanCompose,
    Other,
}

#[derive(Clone, Copy)]
pub(crate) enum Scope {
    Running,
    All,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum EngineKind {
    Podman,
    Docker,
}
