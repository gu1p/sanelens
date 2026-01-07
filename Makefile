.PHONY: build release test fmt clippy lint package install

BIN_NAME ?= composeui
DIST_DIR ?= dist
VERSION ?= $(shell awk -F '\"' '/^version =/ {print $$2; exit}' Cargo.toml)
GIT_SHA ?= $(shell git rev-parse --short=12 HEAD 2>/dev/null || echo unknown)
BUILD_DATE ?= $(shell date -u +"%Y-%m-%dT%H:%M:%SZ")

TARGET_FLAG :=
TARGET_DIR := target
ifneq ($(strip $(TARGET)),)
TARGET_FLAG := --target $(TARGET)
TARGET_DIR := target/$(TARGET)
endif

build:
	GIT_SHA="$(GIT_SHA)" BUILD_DATE="$(BUILD_DATE)" cargo build $(TARGET_FLAG)

release:
	GIT_SHA="$(GIT_SHA)" BUILD_DATE="$(BUILD_DATE)" cargo build --release $(TARGET_FLAG)

test:
	cargo test

fmt:
	cargo fmt --all

clippy:
	cargo clippy --all-targets -- -D warnings

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
	sh ./get-composeui.sh
