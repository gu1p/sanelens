use std::time::{Duration, Instant};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Decision {
    StartNew,
    NoOpinion,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Ruling {
    pub(crate) decision: Decision,
    pub(crate) complete: bool,
}

pub(crate) struct AggregatedEvent {
    pub(crate) line: String,
    pub(crate) container_ts: Option<String>,
}

pub(crate) struct LineView<'a> {
    pub(crate) content: &'a str,
}

impl<'a> LineView<'a> {
    pub(crate) fn new(content: &'a str) -> Self {
        Self { content }
    }
}

struct Vote {
    decision: Decision,
    complete: bool,
}

impl Vote {
    fn start(complete: bool) -> Self {
        Self {
            decision: Decision::StartNew,
            complete,
        }
    }
}

trait Classifier: Send + Sync {
    fn classify(&self, view: &LineView) -> Option<Vote>;
}

pub(crate) struct Router {
    start_classifiers: Vec<Box<dyn Classifier>>,
}

impl Router {
    pub(crate) fn new() -> Self {
        Self {
            start_classifiers: vec![Box::new(JsonClassifier), Box::new(TokenSignalClassifier)],
        }
    }

    pub(crate) fn classify(&self, view: &LineView) -> Ruling {
        for classifier in &self.start_classifiers {
            if let Some(vote) = classifier.classify(view) {
                return Ruling {
                    decision: vote.decision,
                    complete: vote.complete,
                };
            }
        }
        Ruling {
            decision: Decision::NoOpinion,
            complete: false,
        }
    }
}

pub(crate) struct MultilineAggregator {
    router: Router,
    buffer: String,
    last_ingest: Option<Instant>,
    max_gap: Duration,
    current_container_ts: Option<String>,
    last_outer_ts: Option<i64>,
}

impl MultilineAggregator {
    pub(crate) fn new(max_gap: Duration) -> Self {
        Self {
            router: Router::new(),
            buffer: String::new(),
            last_ingest: None,
            max_gap,
            current_container_ts: None,
            last_outer_ts: None,
        }
    }

    pub(crate) fn push_line(&mut self, line: &str, now: Instant) -> Vec<AggregatedEvent> {
        let mut flushed = Vec::new();
        let (container_ts, content, current_outer_ts) = extract_outer_timestamp(line);
        let arrival_gap_exceeded = self
            .last_ingest
            .map(|last| now.duration_since(last) > self.max_gap)
            .unwrap_or(false);
        let gap_exceeded = match (self.last_outer_ts, current_outer_ts) {
            (Some(prev), Some(curr)) if curr >= prev => {
                let delta_ms = curr - prev;
                delta_ms > self.max_gap.as_millis() as i64
            }
            _ => arrival_gap_exceeded,
        };

        let view = LineView::new(content);
        let ruling = self.router.classify(&view);
        let is_start = ruling.decision == Decision::StartNew;

        if gap_exceeded || is_start {
            if let Some(event) = self.take_event() {
                flushed.push(event);
            }
            self.start_new_entry(content, container_ts);
            if ruling.complete {
                if let Some(event) = self.take_event() {
                    flushed.push(event);
                }
            }
            self.last_ingest = Some(now);
            if let Some(ts) = current_outer_ts {
                self.last_outer_ts = Some(ts);
            }
            return flushed;
        }

        if self.buffer.is_empty() {
            self.start_new_entry(content, container_ts);
        } else {
            self.append_line(content);
        }
        self.last_ingest = Some(now);
        if let Some(ts) = current_outer_ts {
            self.last_outer_ts = Some(ts);
        }
        flushed
    }

    pub(crate) fn flush(&mut self) -> Option<AggregatedEvent> {
        self.take_event()
    }

    fn take_event(&mut self) -> Option<AggregatedEvent> {
        if self.buffer.is_empty() {
            None
        } else {
            Some(AggregatedEvent {
                line: std::mem::take(&mut self.buffer),
                container_ts: self.current_container_ts.take(),
            })
        }
    }

    fn start_new_entry(&mut self, line: &str, container_ts: Option<&str>) {
        self.current_container_ts = container_ts.map(|value| value.to_string());
        self.buffer.push_str(line);
    }

    fn append_line(&mut self, line: &str) {
        if !self.buffer.is_empty() {
            self.buffer.push('\n');
        }
        self.buffer.push_str(line);
    }
}

struct JsonClassifier;

impl Classifier for JsonClassifier {
    fn classify(&self, view: &LineView) -> Option<Vote> {
        let candidate = extract_json_candidate(view.content)?;
        if serde_json::from_str::<serde_json::Value>(candidate).is_ok() {
            return Some(Vote::start(true));
        }
        None
    }
}

struct TokenSignalClassifier;

impl Classifier for TokenSignalClassifier {
    fn classify(&self, view: &LineView) -> Option<Vote> {
        if has_start_signal(view.content) {
            return Some(Vote::start(false));
        }
        None
    }
}

fn extract_json_candidate(value: &str) -> Option<&str> {
    let candidate = value.trim();
    let bytes = candidate.as_bytes();
    let start = *bytes.first()?;
    let end = *bytes.last()?;
    if (start == b'{' && end == b'}') || (start == b'[' && end == b']') {
        Some(candidate)
    } else {
        None
    }
}

fn extract_outer_timestamp(line: &str) -> (Option<&str>, &str, Option<i64>) {
    if let Some(split) = line.find(|ch: char| ch.is_whitespace()) {
        let ts = &line[..split];
        if let Some(parsed) = parse_rfc3339_to_epoch_millis(ts) {
            return (Some(ts), &line[split + 1..], Some(parsed));
        }
        return (None, line, None);
    }

    if let Some(parsed) = parse_rfc3339_to_epoch_millis(line) {
        return (Some(line), "", Some(parsed));
    }
    (None, line, None)
}

fn has_start_signal(line: &str) -> bool {
    let tokens: Vec<&str> = line.split_whitespace().take(LEADING_TOKEN_LIMIT).collect();
    if tokens.is_empty() {
        return false;
    }
    if tokens.iter().any(|token| token_contains_datetime(token)) {
        return true;
    }
    for idx in 0..tokens.len().saturating_sub(1) {
        if token_contains_date(tokens[idx]) && token_contains_time(tokens[idx + 1]) {
            return true;
        }
    }
    tokens.iter().any(|token| token_has_severity(token))
}

fn token_has_severity(token: &str) -> bool {
    let bytes = token.as_bytes();
    let mut idx = 0;
    while idx < bytes.len() {
        while idx < bytes.len() && !bytes[idx].is_ascii_alphabetic() {
            idx += 1;
        }
        let start = idx;
        while idx < bytes.len() && bytes[idx].is_ascii_alphabetic() {
            idx += 1;
        }
        if start < idx {
            let word = &token[start..idx];
            if is_level(word) {
                return true;
            }
        }
    }
    false
}

fn token_contains_datetime(token: &str) -> bool {
    let bytes = token.as_bytes();
    let mut idx = 0;
    while idx + 10 < bytes.len() {
        if let Some(end) = match_date_at(bytes, idx) {
            if end < bytes.len() && matches!(bytes[end], b'T' | b't') {
                if match_time_at(bytes, end + 1).is_some() {
                    return true;
                }
            }
        }
        idx += 1;
    }
    false
}

fn token_contains_date(token: &str) -> bool {
    let bytes = token.as_bytes();
    let mut idx = 0;
    while idx + 9 < bytes.len() {
        if match_date_at(bytes, idx).is_some() {
            return true;
        }
        idx += 1;
    }
    false
}

fn token_contains_time(token: &str) -> bool {
    let bytes = token.as_bytes();
    let mut idx = 0;
    while idx + 7 < bytes.len() {
        if match_time_at(bytes, idx).is_some() {
            return true;
        }
        idx += 1;
    }
    false
}

fn match_date_at(bytes: &[u8], idx: usize) -> Option<usize> {
    if idx + 9 >= bytes.len() {
        return None;
    }
    if !is_digit(bytes, idx)
        || !is_digit(bytes, idx + 1)
        || !is_digit(bytes, idx + 2)
        || !is_digit(bytes, idx + 3)
    {
        return None;
    }
    let sep = bytes[idx + 4];
    if sep != b'-' && sep != b'/' {
        return None;
    }
    if !is_digit(bytes, idx + 5) || !is_digit(bytes, idx + 6) {
        return None;
    }
    let sep2 = bytes[idx + 7];
    if sep2 != b'-' && sep2 != b'/' {
        return None;
    }
    if !is_digit(bytes, idx + 8) || !is_digit(bytes, idx + 9) {
        return None;
    }
    Some(idx + 10)
}

fn match_time_at(bytes: &[u8], idx: usize) -> Option<usize> {
    if idx + 7 >= bytes.len() {
        return None;
    }
    if !is_digit(bytes, idx)
        || !is_digit(bytes, idx + 1)
        || bytes[idx + 2] != b':'
        || !is_digit(bytes, idx + 3)
        || !is_digit(bytes, idx + 4)
        || bytes[idx + 5] != b':'
        || !is_digit(bytes, idx + 6)
        || !is_digit(bytes, idx + 7)
    {
        return None;
    }
    let mut end = idx + 8;
    if end < bytes.len() && matches!(bytes[end], b'.' | b',') {
        end += 1;
        let start = end;
        while end < bytes.len() && is_digit(bytes, end) {
            end += 1;
        }
        if start == end {
            return None;
        }
    }
    if end < bytes.len() {
        match bytes[end] {
            b'Z' | b'z' => end += 1,
            b'+' | b'-' => {
                if end + 5 < bytes.len()
                    && is_digit(bytes, end + 1)
                    && is_digit(bytes, end + 2)
                    && bytes[end + 3] == b':'
                    && is_digit(bytes, end + 4)
                    && is_digit(bytes, end + 5)
                {
                    end += 6;
                }
            }
            _ => {}
        }
    }
    Some(end)
}

fn is_digit(bytes: &[u8], idx: usize) -> bool {
    bytes.get(idx).map(|b| b.is_ascii_digit()).unwrap_or(false)
}

fn parse_rfc3339_to_epoch_millis(value: &str) -> Option<i64> {
    let parsed = OffsetDateTime::parse(value, &Rfc3339).ok()?;
    let seconds = parsed.unix_timestamp();
    let millis = i64::from(parsed.millisecond());
    Some(seconds.saturating_mul(1000).saturating_add(millis))
}

fn is_level(value: &str) -> bool {
    LEVELS.iter().any(|level| value.eq_ignore_ascii_case(level))
}

const LEADING_TOKEN_LIMIT: usize = 5;
const LEVELS: [&str; 9] = [
    "TRACE", "DEBUG", "INFO", "WARN", "WARNING", "ERROR", "FATAL", "CRITICAL", "PANIC",
];
