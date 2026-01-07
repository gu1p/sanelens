# composeui

`composeui` is a small Rust wrapper around Docker/Podman Compose that adds a local log UI and
stream-friendly log output while keeping Compose's CLI behavior.

## Features

- Works with `docker compose`, `docker-compose`, `podman compose`, and `podman-compose`
- Opens a local log UI on `up` (can be disabled)
- Colored, prefixed service logs with optional timestamps
- Adds `--build` and `--remove-orphans` on `up` unless explicitly disabled

## Install

### Prebuilt (one line)

```bash
curl -fsSL https://raw.githubusercontent.com/gu1p/composer-ui-rs/main/get-composeui.sh | sh
```

Or with wget:

```bash
wget -qO- https://raw.githubusercontent.com/gu1p/composer-ui-rs/main/get-composeui.sh | sh
```

Overrides:

- `COMPOSEUI_VERSION=0.1.0` (default: latest)
- `COMPOSEUI_INSTALL_DIR=~/.local/bin`
- `COMPOSEUI_REPO=owner/repo`

### From source

```bash
cargo build --release
```

Binary is at `target/release/composeui`.

## Usage

The tool mirrors Compose subcommands. A compose file is required via `-f/--file` or `COMPOSE_FILE`.

```bash
composeui --version
composeui -f docker-compose.yml up
composeui -f docker-compose.yml up -d
COMPOSE_FILE=docker-compose.yml composeui logs
```

When running `up`, a log UI is started on a random local port and printed to stdout.
`composeui --version` prints the build version, commit hash, and build date.

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
make install
```

Package an artifact (uses the host target by default):

```bash
make package
```

Artifacts are written to `dist/` as `composeui-<version>-<target>`.

To package a specific target:

```bash
make package TARGET=x86_64-unknown-linux-gnu
```

You may need to install the target first:

```bash
rustup target add x86_64-unknown-linux-gnu
```

## License

MIT. See `LICENSE`.
