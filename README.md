# composer-ui-rs

`composer-ui-rs` is a small Rust wrapper around Docker/Podman Compose that adds a local log UI and
stream-friendly log output while keeping Compose's CLI behavior.

## Features

- Works with `docker compose`, `docker-compose`, `podman compose`, and `podman-compose`
- Opens a local log UI on `up` (can be disabled)
- Colored, prefixed service logs with optional timestamps
- Adds `--build` and `--remove-orphans` on `up` unless explicitly disabled

## Install

From source:

```bash
cargo build --release
```

Binary is at `target/release/composer-ui-rs`.

## Usage

The tool mirrors Compose subcommands. A compose file is required via `-f/--file` or `COMPOSE_FILE`.

```bash
composer-ui-rs -f docker-compose.yml up
composer-ui-rs -f docker-compose.yml up -d
COMPOSE_FILE=docker-compose.yml composer-ui-rs logs
```

When running `up`, a log UI is started on a random local port and printed to stdout.

## Environment variables

- `COMPOSE_FILE`: compose file path (first entry used if multiple)
- `COMPOSE_PROJECT_NAME`: override project name
- `COMPOSE_CMD`: override the compose command (e.g. `docker compose`)
- `PODMAN_CONNECTION`: podman connection name (when using podman)
- `COMPOSE_LOG_UI`: set to `0/false/no` to disable the log UI
- `COMPOSE_LOG_COLOR`: set to `0/false/no` to disable log colors
- `COMPOSE_LOG_TIMESTAMPS`: set to `0/false/no` to disable log timestamps
- `COMPOSE_DEFAULT_BUILD`: set to `0/false/no` to skip auto `--build` on `up`
- `COMPOSE_DEFAULT_REMOVE_ORPHANS`: set to `0/false/no` to skip auto `--remove-orphans` on `up`

## Development

```bash
make build
make release
make test
```

Package an artifact (uses the host target by default):

```bash
make package
```

To package a specific target:

```bash
make package TARGET=x86_64-unknown-linux-gnu
```

You may need to install the target first:

```bash
rustup target add x86_64-unknown-linux-gnu
```
