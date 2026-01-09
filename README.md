# compose-ui

`compose-ui` is a small Rust wrapper around Docker/Podman Compose that adds a local log UI and
stream-friendly log output while keeping Compose's CLI behavior.

## Features

- Works with `docker compose`, `docker-compose`, `podman compose`, and `podman-compose`
- Opens a local log UI on `up` (can be disabled)
- Colored, prefixed service logs with optional timestamps
- Adds `--build` on `up` and `--remove-orphans` on `up`/`down` unless explicitly disabled

## Install

### Prebuilt (one line)

```bash
curl -fsSL https://raw.githubusercontent.com/gu1p/compose-ui/main/get-compose-ui.sh | sh
```

Or with wget:

```bash
wget -qO- https://raw.githubusercontent.com/gu1p/compose-ui/main/get-compose-ui.sh | sh
```

Overrides:

- `COMPOSE_UI_VERSION=0.1.0` (default: latest)
- `COMPOSE_UI_INSTALL_DIR=~/.local/bin`
- `COMPOSE_UI_REPO=owner/repo`

### From source

```bash
make release
```

Binary is at `target/release/compose-ui`.

If you want to use Cargo directly, build the UI first:

```bash
make -C assets/compose-ui dist
COMPOSE_UI_DIST_DIR=assets/compose-ui/dist cargo build --release
```

To use a prebuilt dist from another stage/location:

```bash
COMPOSE_UI_DIST_DIR=/path/to/dist cargo build --release
```

If you already have `assets/compose-ui/dist` in place, you can skip the UI step in Make:

```bash
UI_BUILD=0 make release
```

## Usage

The tool mirrors Compose subcommands. A compose file is required via `-f/--file` or `COMPOSE_FILE`.

```bash
compose-ui --version
compose-ui -f docker-compose.yml up
compose-ui -f docker-compose.yml up -d
compose-ui -f docker-compose.yml up --no-cache
compose-ui -f docker-compose.yml up --force-recreate
COMPOSE_FILE=docker-compose.yml compose-ui logs
```

When running `up`, a log UI is started on a random local port and printed to stdout.
Passing `--no-cache` to `up` runs a `compose build --no-cache` before starting containers.
Passing `--force-recreate` to `up` forces containers to be recreated, and can be combined with `--no-cache`.
`compose-ui --version` prints the build version, commit hash, and build date.

## Environment variables

- `COMPOSE_FILE`: compose file path (first entry used if multiple)
- `COMPOSE_PROJECT_NAME`: override project name
- `COMPOSE_CMD`: override the compose command (e.g. `docker compose`)
- `PODMAN_CONNECTION`: podman connection name (when using podman)
- `COMPOSE_LOG_UI`: set to `0/false/no` to disable the log UI
- `COMPOSE_LOG_COLOR`: set to `0/false/no` to disable log colors
- `COMPOSE_LOG_TIMESTAMPS`: set to `0/false/no` to disable log timestamps
- `COMPOSE_DEFAULT_BUILD`: set to `0/false/no` to skip auto `--build` on `up`
- `COMPOSE_DEFAULT_REMOVE_ORPHANS`: set to `0/false/no` to skip auto `--remove-orphans` on `up`/`down`

## Development

```bash
make build
make release
make test
make install
```

The UI build is handled by `assets/compose-ui/Makefile` and outputs a standalone `dist/`.

Package an artifact (uses the host target by default):

```bash
make package
```

Artifacts are written to `dist/` as `compose-ui-<version>-<target>`.

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
