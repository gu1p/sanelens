use super::multiline::{AggregatedEvent, MultilineAggregator};
use std::time::{Duration, Instant};

fn collect_events(lines: &[&str]) -> Vec<AggregatedEvent> {
    let mut agg = MultilineAggregator::new(Duration::from_millis(1500));
    let mut now = Instant::now();
    let mut output = Vec::new();
    for line in lines {
        now = now + Duration::from_millis(10);
        output.extend(agg.push_line(line, now));
    }
    if let Some(last) = agg.flush() {
        output.push(last);
    }
    output
}

fn split_outer(line: &str) -> (&str, &str) {
    line.split_once(' ').expect("outer timestamp")
}

#[test]
fn json_lines_are_complete_events() {
    let lines = [
        "2026-01-07T22:22:34-03:00 {\"level\":30,\"time\":1767835354579,\"pid\":1,\"hostname\":\"909a06a70b62\",\"requestId\":\"5b29f50d-41f5-4d75-b18b-d4158aabbd4d\",\"method\":\"GET\",\"path\":\"/healthz\",\"status\":200,\"contentLength\":\"11\",\"durationMs\":0.298627,\"outcome\":\"aborted\",\"msg\":\"Request completed\"}",
        "2026-01-07T22:22:45-03:00 {\"level\":30,\"time\":1767835365603,\"pid\":1,\"hostname\":\"909a06a70b62\",\"requestId\":\"87baa6c0-4e75-43c6-8a99-e554ae0d8f1e\",\"method\":\"GET\",\"path\":\"/healthz\",\"headers\":{},\"hasBody\":false,\"msg\":\"Request received\"}",
    ];

    let events = collect_events(&lines);
    let (ts0, body0) = split_outer(lines[0]);
    let (ts1, body1) = split_outer(lines[1]);

    assert_eq!(events.len(), 2);
    assert_eq!(events[0].line, body0);
    assert_eq!(events[0].container_ts.as_deref(), Some(ts0));
    assert_eq!(events[1].line, body1);
    assert_eq!(events[1].container_ts.as_deref(), Some(ts1));
}

#[test]
fn python_traceback_groups_until_next_start() {
    let lines = [
        "ERROR:    Traceback (most recent call last):",
        "  File \"/app/.venv/lib/python3.11/site-packages/aiormq/connection.py\", line 457, in connect",
        "    reader, writer = await asyncio.open_connection(",
        "                     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^",
        "ConnectionRefusedError: [Errno 111] Connection refused",
        "",
        "The above exception was the direct cause of the following exception:",
        "",
        "Traceback (most recent call last):",
        "  File \"/app/.venv/lib/python3.11/site-packages/starlette/routing.py\", line 732, in lifespan",
        "ERROR:    Application startup failed. Exiting.",
    ];

    let events = collect_events(&lines);

    let expected_first = [
        "ERROR:    Traceback (most recent call last):",
        "  File \"/app/.venv/lib/python3.11/site-packages/aiormq/connection.py\", line 457, in connect",
        "    reader, writer = await asyncio.open_connection(",
        "                     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^",
        "ConnectionRefusedError: [Errno 111] Connection refused",
        "",
        "The above exception was the direct cause of the following exception:",
        "",
        "Traceback (most recent call last):",
        "  File \"/app/.venv/lib/python3.11/site-packages/starlette/routing.py\", line 732, in lifespan",
    ]
    .join("\n");

    assert_eq!(events.len(), 2);
    assert_eq!(events[0].line, expected_first);
    assert_eq!(events[0].container_ts, None);
    assert_eq!(
        events[1].line,
        "ERROR:    Application startup failed. Exiting."
    );
    assert_eq!(events[1].container_ts, None);
}

#[test]
fn logfmt_lines_stay_separate() {
    let lines = [
        "2026-01-07T22:14:41-03:00 time=2026-01-08T01:14:41.564Z level=INFO msg=\"http request\" component=http request.id=b187b902-96de-405b-9a6f-2246fd3e0fb4 method=GET path=/readyz status=200 duration_ms=0",
        "2026-01-07T22:15:03-03:00 time=2026-01-08T01:15:03.557Z level=INFO msg=\"http request\" component=http request.id=701842ec-0ad3-4b4c-b924-c2c90babd8f8 method=GET path=/readyz status=200 duration_ms=0",
    ];

    let events = collect_events(&lines);
    let (ts0, body0) = split_outer(lines[0]);
    let (ts1, body1) = split_outer(lines[1]);

    assert_eq!(events.len(), 2);
    assert_eq!(events[0].line, body0);
    assert_eq!(events[0].container_ts.as_deref(), Some(ts0));
    assert_eq!(events[1].line, body1);
    assert_eq!(events[1].container_ts.as_deref(), Some(ts1));
}

#[test]
fn banner_block_attaches_to_previous_line() {
    let lines = [
        "time=2026-01-07T22:24:38.674Z level=INFO msg=\"server listening\" service.name=saas-bff-backend addr=:8080",
        " ",
        " ┌───────────────────────────────────────────────────┐ ",
        " │                   Fiber v2.52.9                   │ ",
        " │               http://127.0.0.1:8080               │ ",
        " └───────────────────────────────────────────────────┘ ",
    ];

    let events = collect_events(&lines);

    let expected = [
        "time=2026-01-07T22:24:38.674Z level=INFO msg=\"server listening\" service.name=saas-bff-backend addr=:8080",
        " ",
        " ┌───────────────────────────────────────────────────┐ ",
        " │                   Fiber v2.52.9                   │ ",
        " │               http://127.0.0.1:8080               │ ",
        " └───────────────────────────────────────────────────┘ ",
    ]
    .join("\n");

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].line, expected);
    assert_eq!(events[0].container_ts, None);
}

#[test]
fn docker_timestamp_prefix_does_not_split_traceback() {
    let lines = [
        "2026-01-08T00:32:33-03:00 ERROR:    Traceback (most recent call last):",
        "2026-01-08T00:32:33-03:00   File \"/app/.venv/lib/python3.11/site-packages/aiormq/connection.py\", line 457, in connect",
        "2026-01-08T00:32:33-03:00     reader, writer = await asyncio.open_connection(",
        "2026-01-08T00:32:33-03:00 ConnectionRefusedError: [Errno 111] Connection refused",
    ];

    let events = collect_events(&lines);

    let expected = [
        "ERROR:    Traceback (most recent call last):",
        "  File \"/app/.venv/lib/python3.11/site-packages/aiormq/connection.py\", line 457, in connect",
        "    reader, writer = await asyncio.open_connection(",
        "ConnectionRefusedError: [Errno 111] Connection refused",
    ]
    .join("\n");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].line, expected);
    assert_eq!(
        events[0].container_ts.as_deref(),
        Some("2026-01-08T00:32:33-03:00")
    );
}

#[test]
fn docker_timestamp_only_line_keeps_blank_line() {
    let lines = [
        "2026-01-08T11:11:38-03:00 ERROR:    Traceback (most recent call last):",
        "2026-01-08T11:11:38-03:00   File \"/app/.venv/lib/python3.11/site-packages/aiormq/connection.py\", line 920, in connect",
        "2026-01-08T11:11:38-03:00",
        "2026-01-08T11:11:38-03:00     await connection.connect(client_properties or {})",
        "2026-01-08T11:11:38-03:00   File \"/app/.venv/lib/python3.11/site-packages/aiormq/base.py\", line 164, in wrap",
    ];

    let events = collect_events(&lines);

    let expected = [
        "ERROR:    Traceback (most recent call last):",
        "  File \"/app/.venv/lib/python3.11/site-packages/aiormq/connection.py\", line 920, in connect",
        "",
        "    await connection.connect(client_properties or {})",
        "  File \"/app/.venv/lib/python3.11/site-packages/aiormq/base.py\", line 164, in wrap",
    ]
    .join("\n");

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].line, expected);
    assert_eq!(
        events[0].container_ts.as_deref(),
        Some("2026-01-08T11:11:38-03:00")
    );
}

#[test]
fn docker_timestamp_is_metadata_only() {
    let lines = [
        "2026-01-08T00:32:33-03:00 ERROR first line",
        "2026-01-08T00:32:33-03:00 second line",
        "2026-01-08T00:32:34-03:00 third line",
        "2026-01-08T00:32:33-03:00 fourth line",
    ];

    let events = collect_events(&lines);

    let expected = [
        "ERROR first line",
        "second line",
        "third line",
        "fourth line",
    ]
    .join("\n");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].line, expected);
    assert_eq!(
        events[0].container_ts.as_deref(),
        Some("2026-01-08T00:32:33-03:00")
    );
}

#[test]
fn docker_timestamp_gap_overrides_arrival_gap() {
    let mut agg = MultilineAggregator::new(Duration::from_millis(1));
    let start = Instant::now();
    let mut events = Vec::new();

    events.extend(agg.push_line("2026-01-08T00:32:33-03:00 ERROR first line", start));
    events.extend(agg.push_line(
        "2026-01-08T00:32:33-03:00 second line",
        start + Duration::from_millis(10),
    ));
    if let Some(last) = agg.flush() {
        events.push(last);
    }

    let expected = ["ERROR first line", "second line"].join("\n");

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].line, expected);
    assert_eq!(
        events[0].container_ts.as_deref(),
        Some("2026-01-08T00:32:33-03:00")
    );
}

#[test]
fn bracketed_timestamp_level_groups_following_lines() {
    let lines = [
        "[2026-01-07 23:34:00.000] [DEBUG] This is a debug message",
        "Bla",
        "Bla",
        "Bla",
        "Bla",
        "[2026-01-07 23:34:01.123] [INFO] User logged in { userId: 123, role: 'admin' }",
        "[2026-01-07 23:34:02.456] [ERROR] An unexpected failure occurred",
    ];

    let events = collect_events(&lines);

    let expected_first = [
        "[2026-01-07 23:34:00.000] [DEBUG] This is a debug message",
        "Bla",
        "Bla",
        "Bla",
        "Bla",
    ]
    .join("\n");

    assert_eq!(events.len(), 3);
    assert_eq!(events[0].line, expected_first);
    assert_eq!(events[0].container_ts, None);
    assert_eq!(events[1].line, lines[5]);
    assert_eq!(events[1].container_ts, None);
    assert_eq!(events[2].line, lines[6]);
    assert_eq!(events[2].container_ts, None);
}
