# Day-to-day commands. Run `make help` for the list.

UNAME := $(shell uname -s)
EXT_SUFFIX := $(if $(filter Darwin,$(UNAME)),dylib,so)
EXT_DEBUG := $(CURDIR)/target/debug/librusty_sheet.$(EXT_SUFFIX)
EXT_RELEASE := $(CURDIR)/target/release/librusty_sheet.$(EXT_SUFFIX)
PHP ?= php
ROWS ?= 400000

.PHONY: help build release test test-rust test-php lint fmt php-deps fixtures large-fixture bench read-bench bench-php bench-openspout clean

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-16s\033[0m %s\n", $$1, $$2}'

build: ## Debug build of the extension
	cargo build

release: ## Optimised build of the extension
	cargo build --release

test: test-rust test-php ## Run all tests

test-rust: ## Rust unit tests
	cargo test --workspace

test-php: build php-deps ## PHP tests (Pest) with the debug extension loaded
	cd php && $(PHP) -d extension=$(EXT_DEBUG) vendor/bin/pest

lint: ## clippy (warnings are errors) + rustfmt check + PHP stub syntax
	cargo clippy --workspace --all-targets -- -D warnings
	cargo fmt --all --check
	$(PHP) -l php/rusty_sheet.stub.php

fmt: ## Format Rust code
	cargo fmt --all

php-deps: php/vendor/autoload.php

php/vendor/autoload.php: php/composer.json
	cd php && composer install --no-interaction
	@touch $@

fixtures: ## Regenerate the committed edge-case fixtures
	cargo run -q -p gen_fixture -- edge

large-fixture: ## Generate the synthetic benchmark file (ROWS=400000)
	cargo run -q --release -p gen_fixture -- large --rows $(ROWS)

bench: ## Criterion benchmarks (needs `make large-fixture ROWS=10000`)
	cargo bench -p rusty_sheet_core

read-bench: ## Read the synthetic file in Rust: rows/s and peak memory (ROWS=...)
	cargo build -q --release -p rusty_sheet_core --example read_all
	/usr/bin/time $(if $(filter Darwin,$(UNAME)),-l,-v) target/release/examples/read_all tests/fixtures/generated/large-$(ROWS).xlsx

bench-php: release ## PHP-level read of the synthetic file via the extension (ROWS=...)
	/usr/bin/time $(if $(filter Darwin,$(UNAME)),-l,-v) $(PHP) -d extension=$(EXT_RELEASE) php/bench/read-rusty.php tests/fixtures/generated/large-$(ROWS).xlsx

bench-openspout: php-deps ## OpenSpout baseline read of the synthetic file (ROWS=...)
	$(PHP) php/bench/read-openspout.php tests/fixtures/generated/large-$(ROWS).xlsx

clean: ## Remove build output and generated benchmark files
	cargo clean
	rm -rf tests/fixtures/generated
