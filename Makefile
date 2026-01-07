.PHONY: build release test fmt clippy lint package

BIN_NAME ?= composer-ui-rs
DIST_DIR ?= dist

TARGET_FLAG :=
TARGET_DIR := target
ifneq ($(strip $(TARGET)),)
TARGET_FLAG := --target $(TARGET)
TARGET_DIR := target/$(TARGET)
endif

build:
	cargo build $(TARGET_FLAG)

release:
	cargo build --release $(TARGET_FLAG)

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
	archive="$(DIST_DIR)/$(BIN_NAME)-$$target_name.tar.gz"; \
	tar -C "$(TARGET_DIR)/release" -czf "$$archive" "$(BIN_NAME)"; \
	echo "Created $$archive";
