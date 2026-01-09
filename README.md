# sanelens

`sanelens` is a small Rust wrapper around Docker/Podman Compose that adds a local log UI and
stream-friendly log output while keeping Compose's CLI behavior.

## Features

- Works with `docker compose`, `docker-compose`, `podman compose`, and `podman-compose`
- Opens a local log UI on `up` (can be disabled)
- Colored, prefixed service logs with optional timestamps
- Adds `--build` on `up` and `--remove-orphans` on `up`/`down` unless explicitly disabled

## Install

### Prebuilt (one line)

```bash
curl -fsSL https://raw.githubusercontent.com/gu1p/sanelens/main/get-sanelens.sh | sh
```

Or with wget:

```bash
wget -qO- https://raw.githubusercontent.com/gu1p/sanelens/main/get-sanelens.sh | sh
```

Overrides:

- `SANELENS_VERSION=0.1.0` (default: latest)
- `SANELENS_INSTALL_DIR=~/.local/bin`
- `SANELENS_REPO=owner/repo`

### From source

```bash
make release
```

Binary is at `target/release/sanelens`.

If you want to use Cargo directly, build the UI first:

```bash
make -C assets/sanelens dist
SANELENS_DIST_DIR=assets/sanelens/dist cargo build --release
```

To use a prebuilt dist from another stage/location:

```bash
SANELENS_DIST_DIR=/path/to/dist cargo build --release
```

If you already have `assets/sanelens/dist` in place, you can skip the UI step in Make:

```bash
UI_BUILD=0 make release
```

## Usage

The tool mirrors Compose subcommands. A compose file is required via `-f/--file` or `COMPOSE_FILE`.

```bash
sanelens --version
sanelens -f docker-compose.yml up
sanelens -f docker-compose.yml up -d
sanelens -f docker-compose.yml up --no-cache
sanelens -f docker-compose.yml up --force-recreate
COMPOSE_FILE=docker-compose.yml sanelens logs
```

When running `up`, a log UI is started on a random local port and printed to stdout.
Passing `--no-cache` to `up` runs a `compose build --no-cache` before starting containers.
Passing `--force-recreate` to `up` forces containers to be recreated, and can be combined with `--no-cache`.
`sanelens --version` prints the build version, commit hash, and build date.

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

The UI build is handled by `assets/sanelens/Makefile` and outputs a standalone `dist/`.

Package an artifact (uses the host target by default):

```bash
make package
```

Artifacts are written to `dist/` as `sanelens-<version>-<target>`.

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
