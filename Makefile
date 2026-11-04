.DEFAULT_GOAL := desktop-app

# The CLI and GUI have separate lockfiles. They share one release cache and the
# installed binaries must be siblings for the desktop bridge and its tests.
TARGET_DIR := $(CURDIR)/target
CARGO_RELEASE = nice -n 10 cargo
RELEASE_FLAGS = --release --locked --target-dir "$(TARGET_DIR)"

.PHONY: init harness-ready companion build desktop-app run-desktop test check

init:
	git submodule update --init --recursive

harness-ready:
	@test -f harness/Cargo.toml || { echo "Harness not initialized. Run 'make init'." >&2; exit 1; }

companion: harness-ready
	$(CARGO_RELEASE) build $(RELEASE_FLAGS) --manifest-path harness/Cargo.toml -p pa-cli --bin prime-agent

build: companion
	$(CARGO_RELEASE) build $(RELEASE_FLAGS) -p pa-desktop --bin prime-agent-ui

desktop-app: build
	python3 scripts/package_desktop_app.py --profile release --out-dir dist

run-desktop: build
	PI_PACKAGE_DIR="$(CURDIR)/harness" "$(TARGET_DIR)/release/prime-agent-ui"

test: companion
	$(CARGO_RELEASE) test $(RELEASE_FLAGS) -p pa-desktop

check: harness-ready
	cargo fmt --all --check
	$(CARGO_RELEASE) clippy $(RELEASE_FLAGS) -p pa-desktop --no-deps --all-targets -- -D warnings
	$(MAKE) test
