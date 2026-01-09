.PHONY: build release test fmt clippy lint package install ui-build ui-clean

BIN_NAME ?= compose-ui
DIST_DIR ?= dist
VERSION ?= $(shell awk -F '\"' '/^version =/ {print $$2; exit}' Cargo.toml)
GIT_SHA ?= $(shell git rev-parse --short=12 HEAD 2>/dev/null || echo unknown)
BUILD_DATE ?= $(shell date -u +"%Y-%m-%dT%H:%M:%SZ")
UI_DIR ?= assets/compose-ui
UI_DIST_DIR ?= $(abspath $(UI_DIR)/dist)
UI_LOCK := $(wildcard $(UI_DIR)/package-lock.json)
UI_BUILD ?= 1
BUILD_DEPS :=
ifneq ($(UI_BUILD),0)
BUILD_DEPS += ui-build
endif
UI_SOURCES := $(shell find $(UI_DIR)/src -type f) \
	$(UI_DIR)/index.html \
	$(UI_DIR)/vite.config.ts \
	$(UI_DIR)/tailwind.config.cjs \
	$(UI_DIR)/postcss.config.cjs \
	$(UI_DIR)/svelte.config.js \
	$(UI_DIR)/Makefile \
	$(UI_DIR)/package.json \
	$(UI_LOCK)

TARGET_FLAG :=
TARGET_DIR := target
ifneq ($(strip $(TARGET)),)
TARGET_FLAG := --target $(TARGET)
TARGET_DIR := target/$(TARGET)
endif

build: $(BUILD_DEPS)
	COMPOSE_UI_DIST_DIR="$(UI_DIST_DIR)" \
	GIT_SHA="$(GIT_SHA)" BUILD_DATE="$(BUILD_DATE)" cargo build $(TARGET_FLAG)

release: $(BUILD_DEPS)
	COMPOSE_UI_DIST_DIR="$(UI_DIST_DIR)" \
	GIT_SHA="$(GIT_SHA)" BUILD_DATE="$(BUILD_DATE)" cargo build --release $(TARGET_FLAG)

test: $(BUILD_DEPS)
	COMPOSE_UI_DIST_DIR="$(UI_DIST_DIR)" cargo test

fmt:
	cargo fmt --all

clippy: $(BUILD_DEPS)
	COMPOSE_UI_DIST_DIR="$(UI_DIST_DIR)" cargo clippy --all-targets -- -D warnings

lint: fmt clippy

package: release
	@set -eu; \
	bin_path="$(TARGET_DIR)/release/$(BIN_NAME)"; \
	if [ ! -f "$$bin_path" ]; then \
		echo "Binary not found at $$bin_path" >&2; \
		exit 1; \
	fi; \
	mkdir -p "$(DIST_DIR)"; \
	target_name="$(TARGET)"; \
	if [ -z "$$target_name" ]; then \
		target_name="$$(rustc -vV | awk '/host/ {print $$2}')"; \
	fi; \
	version="$(VERSION)"; \
	sanitized_version="$${version#v}"; \
	out_name="$(BIN_NAME)-$$sanitized_version-$$target_name"; \
	cp "$$bin_path" "$(DIST_DIR)/$$out_name"; \
	chmod +x "$(DIST_DIR)/$$out_name"; \
	echo "Created $(DIST_DIR)/$$out_name";

install:
	sh ./get-compose-ui.sh

ui-build: $(UI_DIST_DIR)/index.html

$(UI_DIST_DIR)/index.html: $(UI_SOURCES)
	$(MAKE) -C "$(UI_DIR)" dist OUT_DIR="$(UI_DIST_DIR)"

ui-clean:
	$(MAKE) -C "$(UI_DIR)" clean OUT_DIR="$(UI_DIST_DIR)"
