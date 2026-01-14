use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};

use crate::domain::traffic::{
    Confidence, Correlation, EntityId, FlowKey, FlowMetrics, FlowObservation, HttpObservation,
    Observation, ObservationAttrs, Peer, Resolver, Socket, Transport, Visibility,
};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

#[derive(Default)]
pub struct EnvoyAccessLog {
    #[allow(dead_code)]
    pub timestamp: Option<String>,
    pub method: Option<String>,
    pub path: Option<String>,
    pub authority: Option<String>,
    #[allow(dead_code)]
    pub protocol: Option<String>,
    pub response_code: Option<u16>,
    pub duration_ms: Option<u64>,
    pub downstream_remote_address: Option<String>,
    pub upstream_host: Option<String>,
    pub bytes_received: Option<u64>,
    pub bytes_sent: Option<u64>,
    pub request_id: Option<String>,
    pub request_user_agent: Option<String>,
    pub request_content_type: Option<String>,
    pub request_accept: Option<String>,
    pub request_forwarded_for: Option<String>,
    pub request_forwarded_proto: Option<String>,
    pub request_body: Option<String>,
    pub response_content_type: Option<String>,
    pub response_content_length: Option<String>,
    pub response_body: Option<String>,
}

struct EnvoyObservationContext<'a> {
    service_name: &'a str,
    resolver: &'a dyn Resolver,
    is_egress: bool,
}

struct EnvoySockets {
    downstream: Option<Socket>,
    upstream: Option<Socket>,
}

struct HttpLogParts {
    method: Option<String>,
    path: Option<String>,
    status: Option<u16>,
    duration_ms: Option<u64>,
    bytes_in: Option<u64>,
    bytes_out: Option<u64>,
    request_id: Option<String>,
    request_headers: BTreeMap<String, String>,
    response_headers: BTreeMap<String, String>,
    request_body: Option<String>,
    response_body: Option<String>,
}

struct RequestHeaderParts {
    authority: Option<String>,
    request_id: Option<String>,
    request_user_agent: Option<String>,
    request_content_type: Option<String>,
    request_accept: Option<String>,
    request_forwarded_for: Option<String>,
    request_forwarded_proto: Option<String>,
}

pub fn parse_envoy_log_line(line: &str) -> Option<EnvoyAccessLog> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let obj = value.as_object()?;
    Some(build_envoy_access_log(obj))
}

fn build_envoy_access_log(obj: &serde_json::Map<String, serde_json::Value>) -> EnvoyAccessLog {
    EnvoyAccessLog {
        timestamp: string_field(obj, "timestamp"),
        method: string_field(obj, "method"),
        path: string_field(obj, "path"),
        authority: string_field(obj, "authority"),
        protocol: string_field(obj, "protocol"),
        response_code: u16_field(obj, "response_code"),
        duration_ms: u64_field(obj, "duration_ms"),
        downstream_remote_address: string_field(obj, "downstream_remote_address"),
        upstream_host: string_field(obj, "upstream_host"),
        bytes_received: u64_field(obj, "bytes_received"),
        bytes_sent: u64_field(obj, "bytes_sent"),
        request_id: string_field(obj, "request_id"),
        request_user_agent: string_field(obj, "request_user_agent"),
        request_content_type: string_field(obj, "request_content_type"),
        request_accept: string_field(obj, "request_accept"),
        request_forwarded_for: string_field(obj, "request_forwarded_for"),
        request_forwarded_proto: string_field(obj, "request_forwarded_proto"),
        request_body: string_field(obj, "request_body"),
        response_content_type: string_field(obj, "response_content_type"),
        response_content_length: string_field(obj, "response_content_length"),
        response_body: string_field(obj, "response_body"),
    }
}

fn string_field(obj: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    obj.get(key)
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
}

fn u64_field(obj: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<u64> {
    obj.get(key).and_then(value_to_u64)
}

fn u16_field(obj: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<u16> {
    obj.get(key)
        .and_then(value_to_u64)
        .and_then(|value| u16::try_from(value).ok())
}

pub fn observation_from_envoy(
    log: EnvoyAccessLog,
    service_name: &str,
    resolver: &dyn Resolver,
    is_egress: bool,
    now_ms: u64,
) -> Option<Observation> {
    let sockets = parse_envoy_sockets(&log);
    let context = EnvoyObservationContext {
        service_name,
        resolver,
        is_egress,
    };
    let (peer, attrs) = resolve_peer_and_attrs(&log, &context, &sockets);

    if attrs.visibility == Visibility::L7Semantics {
        return Some(build_http_observation(
            log,
            peer,
            attrs,
            now_ms,
            context.is_egress,
        ));
    }

    build_flow_observation(&log, peer, attrs, now_ms, &sockets)
}

#[allow(clippy::too_many_lines)]
pub fn observation_from_tap(
    payload: &str,
    service_name: &str,
    resolver: &dyn Resolver,
    is_egress: bool,
    now_ms: u64,
) -> Option<Observation> {
    let value: serde_json::Value = serde_json::from_str(payload).ok()?;
    let wrapper = value.as_object()?;
    let trace = tap_object(wrapper, "http_buffered_trace", "httpBufferedTrace")?;
    let request = tap_object(trace, "request", "request")?;
    let response = tap_object(trace, "response", "response")?;
    let mut request_headers = parse_tap_headers(tap_array(request, "headers", "headers"));
    let response_headers = parse_tap_headers(tap_array(response, "headers", "headers"));
    if !request_headers.contains_key("host") {
        if let Some(authority) = request_headers.get(":authority").cloned() {
            request_headers.insert("host".to_string(), authority);
        }
    }

    let method = header_value(&request_headers, ":method")
        .or_else(|| header_value(&request_headers, "method"));
    let path =
        header_value(&request_headers, ":path").or_else(|| header_value(&request_headers, "path"));
    let authority = header_value(&request_headers, ":authority")
        .or_else(|| header_value(&request_headers, "host"));
    let request_id = header_value(&request_headers, "x-request-id");
    let status = header_value(&response_headers, ":status")
        .or_else(|| header_value(&response_headers, "status"))
        .and_then(|value| value.parse::<u16>().ok());

    let request_content_type = request_headers.get("content-type").cloned();
    let response_content_type = response_headers.get("content-type").cloned();
    let request_body_raw = parse_tap_body(tap_object(request, "body", "body"));
    let response_body_raw = parse_tap_body(tap_object(response, "body", "body"));
    let request_body = normalize_body(request_body_raw.clone(), request_content_type.as_deref());
    let response_body = normalize_body(response_body_raw.clone(), response_content_type.as_deref());
    let bytes_in = parse_content_length(&request_headers)
        .or_else(|| request_body_raw.as_ref().map(|body| body.len() as u64));
    let bytes_out = parse_content_length(&response_headers)
        .or_else(|| response_body_raw.as_ref().map(|body| body.len() as u64));

    let (at_ms, duration_ms) = tap_timing(request, response, now_ms);
    let downstream_socket =
        parse_tap_connection(trace, "downstream_connection", "downstreamConnection");
    let upstream_socket = parse_tap_connection(trace, "upstream_connection", "upstreamConnection");
    let src_entity = downstream_socket
        .as_ref()
        .and_then(|socket| resolver.resolve_entity(socket));
    let dst_entity = if is_egress {
        parse_external_entity(authority.as_deref()).or_else(|| {
            upstream_socket.as_ref().map(|socket| EntityId::External {
                ip: socket.ip,
                dns_name: None,
            })
        })
    } else {
        Some(EntityId::Workload {
            name: service_name.to_string(),
            instance: None,
        })
    };
    let confidence = resolve_confidence(src_entity.as_ref(), dst_entity.as_ref());
    let peer = build_peer(src_entity, dst_entity, downstream_socket, upstream_socket);
    let attrs = ObservationAttrs {
        visibility: Visibility::L7Semantics,
        confidence,
        tags: BTreeMap::default(),
    };

    let path = build_http_path_parts(path, authority.as_deref(), None, is_egress);

    Some(Observation::Http(HttpObservation {
        at_ms,
        peer,
        method,
        path,
        status,
        duration_ms,
        bytes_in,
        bytes_out,
        request_headers,
        response_headers,
        request_body,
        response_body,
        correlation: Correlation {
            request_id,
            ..Default::default()
        },
        attrs,
    }))
}

fn parse_envoy_sockets(log: &EnvoyAccessLog) -> EnvoySockets {
    let downstream = log
        .downstream_remote_address
        .as_deref()
        .and_then(parse_socket);
    let upstream = log.upstream_host.as_deref().and_then(parse_socket);
    EnvoySockets {
        downstream,
        upstream,
    }
}

fn resolve_peer_and_attrs(
    log: &EnvoyAccessLog,
    context: &EnvoyObservationContext<'_>,
    sockets: &EnvoySockets,
) -> (Peer, ObservationAttrs) {
    let src_entity = sockets
        .downstream
        .as_ref()
        .and_then(|socket| context.resolver.resolve_entity(socket));
    let dst_entity = resolve_dst_entity(
        log,
        context.is_egress,
        context.service_name,
        sockets.upstream.as_ref(),
    );
    let confidence = resolve_confidence(src_entity.as_ref(), dst_entity.as_ref());
    let peer = build_peer(
        src_entity,
        dst_entity,
        sockets.downstream.clone(),
        sockets.upstream.clone(),
    );
    let attrs = build_attrs(log, confidence);
    (peer, attrs)
}

fn build_http_observation(
    log: EnvoyAccessLog,
    peer: Peer,
    attrs: ObservationAttrs,
    now_ms: u64,
    is_egress: bool,
) -> Observation {
    let parts = build_http_parts(log, is_egress);
    Observation::Http(HttpObservation {
        at_ms: now_ms,
        peer,
        method: parts.method,
        path: parts.path,
        status: parts.status,
        duration_ms: parts.duration_ms,
        bytes_in: parts.bytes_in,
        bytes_out: parts.bytes_out,
        request_headers: parts.request_headers,
        response_headers: parts.response_headers,
        request_body: parts.request_body,
        response_body: parts.response_body,
        correlation: Correlation {
            request_id: parts.request_id,
            ..Default::default()
        },
        attrs,
    })
}

fn build_http_parts(log: EnvoyAccessLog, is_egress: bool) -> HttpLogParts {
    let EnvoyAccessLog {
        method,
        path,
        authority,
        response_code,
        duration_ms,
        upstream_host,
        bytes_received,
        bytes_sent,
        request_id,
        request_user_agent,
        request_content_type,
        request_accept,
        request_forwarded_for,
        request_forwarded_proto,
        request_body,
        response_content_type,
        response_content_length,
        response_body,
        ..
    } = log;
    let path = build_http_path_parts(
        path,
        authority.as_deref(),
        upstream_host.as_deref(),
        is_egress,
    );
    let request_headers = build_request_headers(RequestHeaderParts {
        authority,
        request_id: request_id.clone(),
        request_user_agent,
        request_content_type: request_content_type.clone(),
        request_accept,
        request_forwarded_for,
        request_forwarded_proto,
    });
    let response_headers =
        build_response_headers_from_parts(response_content_type.clone(), response_content_length);
    let request_body = normalize_body(request_body, request_content_type.as_deref());
    let response_body = normalize_body(response_body, response_content_type.as_deref());

    HttpLogParts {
        method,
        path,
        status: response_code,
        duration_ms,
        bytes_in: bytes_received,
        bytes_out: bytes_sent,
        request_id,
        request_headers,
        response_headers,
        request_body,
        response_body,
    }
}

fn build_request_headers(parts: RequestHeaderParts) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    insert_header(&mut headers, "host", parts.authority);
    insert_header(&mut headers, "x-request-id", parts.request_id);
    insert_header(&mut headers, "user-agent", parts.request_user_agent);
    insert_header(&mut headers, "content-type", parts.request_content_type);
    insert_header(&mut headers, "accept", parts.request_accept);
    insert_header(&mut headers, "x-forwarded-for", parts.request_forwarded_for);
    insert_header(
        &mut headers,
        "x-forwarded-proto",
        parts.request_forwarded_proto,
    );
    headers
}

fn build_response_headers_from_parts(
    response_content_type: Option<String>,
    response_content_length: Option<String>,
) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    insert_header(&mut headers, "content-type", response_content_type);
    insert_header(&mut headers, "content-length", response_content_length);
    headers
}

fn build_flow_observation(
    log: &EnvoyAccessLog,
    peer: Peer,
    attrs: ObservationAttrs,
    now_ms: u64,
    sockets: &EnvoySockets,
) -> Option<Observation> {
    let flow = build_flow_key(
        peer.raw.clone(),
        sockets.downstream.clone(),
        sockets.upstream.clone(),
    )?;
    Some(Observation::Flow(FlowObservation {
        at_ms: now_ms,
        flow,
        metrics: FlowMetrics {
            bytes_in: log.bytes_received,
            bytes_out: log.bytes_sent,
            packets: None,
            duration_ms: log.duration_ms,
        },
        peer,
        attrs,
    }))
}

fn resolve_dst_entity(
    log: &EnvoyAccessLog,
    is_egress: bool,
    service_name: &str,
    upstream: Option<&Socket>,
) -> Option<EntityId> {
    if is_egress {
        parse_external_entity(log.authority.as_deref().or(log.upstream_host.as_deref())).or_else(
            || {
                upstream.map(|socket| EntityId::External {
                    ip: socket.ip,
                    dns_name: None,
                })
            },
        )
    } else {
        Some(EntityId::Workload {
            name: service_name.to_string(),
            instance: None,
        })
    }
}

const fn resolve_confidence(
    src_entity: Option<&EntityId>,
    dst_entity: Option<&EntityId>,
) -> Confidence {
    if src_entity.is_some() && dst_entity.is_some() {
        Confidence::Exact
    } else {
        Confidence::Likely
    }
}

const fn build_peer(
    src: Option<EntityId>,
    dst: Option<EntityId>,
    downstream: Option<Socket>,
    upstream: Option<Socket>,
) -> Peer {
    let raw = match (downstream, upstream) {
        (Some(src), Some(dst)) => Some(FlowKey {
            src,
            dst,
            transport: Transport::Tcp,
        }),
        _ => None,
    };
    Peer { src, dst, raw }
}

fn build_attrs(log: &EnvoyAccessLog, confidence: Confidence) -> ObservationAttrs {
    let visibility = if log.method.is_some() || log.path.is_some() || log.authority.is_some() {
        Visibility::L7Semantics
    } else {
        Visibility::L4Flow
    };
    ObservationAttrs {
        visibility,
        confidence,
        tags: BTreeMap::default(),
    }
}

fn build_http_path_parts(
    path: Option<String>,
    authority: Option<&str>,
    upstream_host: Option<&str>,
    is_egress: bool,
) -> Option<String> {
    if !is_egress {
        return path;
    }
    let authority = authority.or(upstream_host);
    if let Some(authority) = authority {
        return path.map_or_else(
            || Some(authority.to_string()),
            |path| Some(format!("{authority}{path}")),
        );
    }
    path
}

fn build_flow_key(
    peer_raw: Option<FlowKey>,
    downstream: Option<Socket>,
    upstream: Option<Socket>,
) -> Option<FlowKey> {
    if let Some(flow) = peer_raw {
        return Some(flow);
    }
    let src = downstream?;
    let dst = upstream?;
    Some(FlowKey {
        src,
        dst,
        transport: Transport::Tcp,
    })
}

fn parse_socket(raw: &str) -> Option<Socket> {
    if let Ok(sock) = raw.parse::<SocketAddr>() {
        return Some(Socket {
            ip: sock.ip(),
            port: sock.port(),
        });
    }
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let (host, port) = raw.rsplit_once(':')?;
    let host = host.trim_matches(['[', ']']);
    let ip = host.parse::<IpAddr>().ok()?;
    let port = port.parse::<u16>().ok()?;
    Some(Socket { ip, port })
}

fn parse_tap_headers(entries: Option<&Vec<serde_json::Value>>) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    let Some(entries) = entries else {
        return headers;
    };
    for entry in entries {
        let Some(obj) = entry.as_object() else {
            continue;
        };
        let key = obj.get("key").and_then(|value| value.as_str());
        let value = obj.get("value").and_then(|value| value.as_str());
        let (Some(key), Some(value)) = (key, value) else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        if key.is_empty() {
            continue;
        }
        let entry = headers.entry(key).or_insert_with(String::new);
        if !entry.is_empty() {
            entry.push_str(", ");
        }
        entry.push_str(value);
    }
    headers
}

fn parse_tap_body(body: Option<&serde_json::Map<String, serde_json::Value>>) -> Option<String> {
    let body = body?;
    let value = tap_string(body, "as_string", "asString")?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let truncated = tap_bool(body, "truncated", "truncated").unwrap_or(false);
    if truncated {
        Some(format!("{value}\n... (truncated by tap)"))
    } else {
        Some(value.to_string())
    }
}

fn parse_tap_connection(
    trace: &serde_json::Map<String, serde_json::Value>,
    snake: &str,
    camel: &str,
) -> Option<Socket> {
    let connection = tap_object(trace, snake, camel)?;
    let remote = tap_object(connection, "remote_address", "remoteAddress")?;
    let socket = tap_object(remote, "socket_address", "socketAddress")?;
    let address = tap_string(socket, "address", "address")?;
    let port =
        tap_u64(socket, "port_value", "portValue").and_then(|value| u16::try_from(value).ok())?;
    let ip = address.parse::<IpAddr>().ok()?;
    Some(Socket { ip, port })
}

fn tap_timing(
    request: &serde_json::Map<String, serde_json::Value>,
    response: &serde_json::Map<String, serde_json::Value>,
    now_ms: u64,
) -> (u64, Option<u64>) {
    let request_ts = tap_timestamp_ms(request, "headers_received_time", "headersReceivedTime");
    let response_ts = tap_timestamp_ms(response, "headers_received_time", "headersReceivedTime");
    let at_ms = request_ts.unwrap_or(now_ms);
    let duration_ms = match (request_ts, response_ts) {
        (Some(start), Some(end)) if end >= start => Some(end - start),
        _ => None,
    };
    (at_ms, duration_ms)
}

fn tap_timestamp_ms(
    obj: &serde_json::Map<String, serde_json::Value>,
    snake: &str,
    camel: &str,
) -> Option<u64> {
    let value = tap_value(obj, snake, camel)?;
    match value {
        serde_json::Value::String(value) => parse_rfc3339_ms(value),
        serde_json::Value::Object(map) => {
            let seconds = tap_i64(map, "seconds", "seconds")?;
            let nanos = tap_i64(map, "nanos", "nanos").unwrap_or(0);
            if seconds < 0 {
                return None;
            }
            let millis = seconds
                .saturating_mul(1000)
                .saturating_add(nanos / 1_000_000);
            Some(u64::try_from(millis).ok()?)
        }
        _ => None,
    }
}

fn parse_rfc3339_ms(value: &str) -> Option<u64> {
    let parsed = OffsetDateTime::parse(value, &Rfc3339).ok()?;
    let seconds = parsed.unix_timestamp();
    if seconds < 0 {
        return None;
    }
    let millis = seconds
        .saturating_mul(1000)
        .saturating_add(i64::from(parsed.millisecond()));
    u64::try_from(millis).ok()
}

fn tap_value<'a>(
    obj: &'a serde_json::Map<String, serde_json::Value>,
    snake: &str,
    camel: &str,
) -> Option<&'a serde_json::Value> {
    obj.get(snake).or_else(|| obj.get(camel))
}

fn tap_object<'a>(
    obj: &'a serde_json::Map<String, serde_json::Value>,
    snake: &str,
    camel: &str,
) -> Option<&'a serde_json::Map<String, serde_json::Value>> {
    tap_value(obj, snake, camel)?.as_object()
}

fn tap_array<'a>(
    obj: &'a serde_json::Map<String, serde_json::Value>,
    snake: &str,
    camel: &str,
) -> Option<&'a Vec<serde_json::Value>> {
    tap_value(obj, snake, camel)?.as_array()
}

fn tap_string<'a>(
    obj: &'a serde_json::Map<String, serde_json::Value>,
    snake: &str,
    camel: &str,
) -> Option<&'a str> {
    tap_value(obj, snake, camel)?.as_str()
}

fn tap_bool(
    obj: &serde_json::Map<String, serde_json::Value>,
    snake: &str,
    camel: &str,
) -> Option<bool> {
    tap_value(obj, snake, camel)?.as_bool()
}

fn tap_i64(
    obj: &serde_json::Map<String, serde_json::Value>,
    snake: &str,
    camel: &str,
) -> Option<i64> {
    if let Some(value) = tap_value(obj, snake, camel)?.as_i64() {
        return Some(value);
    }
    tap_value(obj, snake, camel)?
        .as_str()
        .and_then(|value| value.parse::<i64>().ok())
}

fn tap_u64(
    obj: &serde_json::Map<String, serde_json::Value>,
    snake: &str,
    camel: &str,
) -> Option<u64> {
    if let Some(value) = tap_value(obj, snake, camel)?.as_u64() {
        return Some(value);
    }
    tap_value(obj, snake, camel)?
        .as_str()
        .and_then(|value| value.parse::<u64>().ok())
}

fn header_value(headers: &BTreeMap<String, String>, key: &str) -> Option<String> {
    headers.get(&key.to_ascii_lowercase()).cloned()
}

fn parse_content_length(headers: &BTreeMap<String, String>) -> Option<u64> {
    let value = headers.get("content-length")?;
    let value = value.split(',').next()?.trim();
    value.parse::<u64>().ok()
}

fn parse_external_entity(raw: Option<&str>) -> Option<EntityId> {
    let raw = raw?;
    let value = raw.trim();
    if value.is_empty() {
        return None;
    }
    let (host, _) = value.rsplit_once(':').unwrap_or((value, ""));
    let host = host.trim_matches(['[', ']']);
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Some(EntityId::External { ip, dns_name: None });
    }
    Some(EntityId::External {
        ip: IpAddr::from([0, 0, 0, 0]),
        dns_name: Some(host.to_string()),
    })
}

fn value_to_u64(value: &serde_json::Value) -> Option<u64> {
    if let Some(value) = value.as_u64() {
        return Some(value);
    }
    if let Some(value) = value.as_str() {
        return value.parse::<u64>().ok();
    }
    None
}

fn insert_header(headers: &mut BTreeMap<String, String>, key: &str, value: Option<String>) {
    let Some(value) = normalize_header_value(value) else {
        return;
    };
    headers.insert(key.to_string(), value);
}

const NON_JSON_BODY_PREVIEW_LIMIT: usize = 4096;

fn normalize_body(body: Option<String>, content_type: Option<&str>) -> Option<String> {
    let body = body?;
    let trimmed = body.trim();
    if trimmed.is_empty() || trimmed == "-" {
        return None;
    }
    let content_type = content_type.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(
                trimmed
                    .split(';')
                    .next()
                    .unwrap_or(trimmed)
                    .trim()
                    .to_string(),
            )
        }
    });
    if is_json_content_type(content_type.as_deref()) {
        return Some(body);
    }
    let (snippet, truncated) = truncate_body(&body, NON_JSON_BODY_PREVIEW_LIMIT);
    if truncated {
        Some(format!("{snippet}\n... (cropped)"))
    } else {
        Some(snippet)
    }
}

fn is_json_content_type(content_type: Option<&str>) -> bool {
    content_type.is_some_and(|value| value.to_ascii_lowercase().contains("json"))
}

fn truncate_body(body: &str, max_bytes: usize) -> (String, bool) {
    if body.len() <= max_bytes {
        return (body.to_string(), false);
    }
    let mut end = 0;
    for (idx, ch) in body.char_indices() {
        if idx >= max_bytes {
            break;
        }
        end = idx + ch.len_utf8();
    }
    (body[..end].to_string(), true)
}

fn normalize_header_value(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() || trimmed == "-" {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}
