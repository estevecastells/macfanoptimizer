.PHONY: build test lint fmt app install uninstall run-app benchmark ci clean

build:            ## Build the daemon and CLI (release)
	cargo build --release --locked

test:             ## Rust unit/integration/simulator tests + Swift protocol checks
	cargo test --locked
	cd app && swift run -c debug KitChecks

lint:             ## Formatting and lints, exactly as CI runs them
	cargo fmt --all --check
	cargo clippy --locked --all-targets -- -D warnings

fmt:
	cargo fmt --all

app:              ## Build build/MacFanOptimizer.app
	scripts/build-app.sh

install: build    ## Install the daemon as a launchd service (asks for sudo)
	sudo scripts/install.sh --bin-dir target/release

uninstall:        ## Remove the daemon and return fans to macOS
	sudo scripts/uninstall.sh

run-app: app      ## Build and launch the menu bar app
	open build/MacFanOptimizer.app

benchmark: build  ## Compare policies in the thermal simulator
	target/release/fanctl benchmark

ci: lint test app ## Everything a pull request must pass

clean:
	cargo clean
	rm -rf build app/.build

help:
	@grep -E '^[a-z-]+:.*## ' $(MAKEFILE_LIST) | awk -F':.*## ' '{printf "  %-12s %s\n", $$1, $$2}'
