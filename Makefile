.PHONY: help install update update-latest setup dev dev-api dev-web run-api fake-llm migrate seed migrations contracts test test-db test-bruno smoke lint format type-check boundaries check ci build native-dev native-lint native-test native-build android-dev android-build clean

.DEFAULT_GOAL := help

API_ENV := apps/api/.env.local
WITH_API_ENV := bash scripts/with-env.sh $(API_ENV)
CONTRACTS_TS := packages/contracts/ts/generated
CONTRACTS_FIXTURES := packages/contracts/test/fixtures

help:
	@echo "Targets:"
	@echo "  install        Install pnpm and Cargo dependencies"
	@echo "  update         Update dependencies within semver ranges"
	@echo "  update-latest  Update pnpm dependencies to latest stable (Cargo: see README)"
	@echo "  setup          Install, create env files (with a SECRET_KEY), start Postgres, migrate, seed"
	@echo "  dev            Start Postgres, then run API (:3000) + web (:4000)"
	@echo "  dev-api        Start Postgres, then run the API only"
	@echo "  dev-web        Run the web dev server only"
	@echo "  fake-llm       Run the fake model and MCP server (:4010, key fake-key; tools/fake-llm/README.md)"
	@echo "  migrate        Apply migrations + readonly grants"
	@echo "  seed           Apply local SQL seed files"
	@echo "  migrations     Scaffold a migration pair (name=<name>)"
	@echo "  contracts      Regenerate TypeScript bindings and conformance fixtures from Rust contracts"
	@echo "  test           Rust and TypeScript unit tests"
	@echo "  test-db        Tests that need Postgres (ignored by default; one throwaway database each)"
	@echo "  test-bruno     Bruno API tests (starts this checkout's API; BASE_URL=<url> tests a running one)"
	@echo "  smoke          The API against fake-llm over HTTP, on a throwaway database (needs Postgres)"
	@echo "  lint           rustfmt, clippy, Rust import paths, Biome, markdownlint"
	@echo "  format         Auto-fix formatting and lint"
	@echo "  type-check     cargo check + tsc across the workspace"
	@echo "  boundaries     Enforce crate dependency rules"
	@echo "  check          format + lint + type-check + test + boundaries"
	@echo "  ci             check without writes, plus contract drift"
	@echo "  build          Release API binaries + production web bundle"
	@echo "  native-dev     Desktop app on the web dev server (needs WebKitGTK; apps/native/README.md)"
	@echo "  native-lint    rustfmt and Clippy for the Tauri shell (needs WebKitGTK)"
	@echo "  native-test    Rust unit tests of the Tauri shell (needs WebKitGTK)"
	@echo "  native-build   Linux .deb of the desktop app (needs WebKitGTK)"
	@echo "  android-dev    Android app on a device or emulator (needs the Android SDK and NDK)"
	@echo "  android-build  Debug-signed arm64 APK (needs the Android SDK and NDK)"
	@echo "  clean          Wipe build output, dependencies, and the database volume"

install:
	pnpm install
	cargo fetch

update:
	pnpm update
	cargo update

update-latest:
	pnpm update-latest
	cargo update

setup: install
	bash scripts/env-local.sh
	pnpm db:start
	$(MAKE) --no-print-directory migrate
	$(MAKE) --no-print-directory seed

dev:
	pnpm db:start
	$(MAKE) --no-print-directory -j 2 run-api dev-web

dev-api:
	pnpm db:start
	$(MAKE) --no-print-directory run-api

dev-web:
	pnpm --filter @copilot/web dev

run-api:
	bash scripts/dev-api.sh

fake-llm:
	cargo run -q -p fake-llm

migrate:
	$(WITH_API_ENV) cargo run -q -p copilot-database --bin migrate

seed:
	$(WITH_API_ENV) cargo run -q -p copilot-database --bin seed

migrations:
	bash packages/database/scripts/generate-migration.sh $(name)

contracts:
	rm -rf $(CONTRACTS_TS)
	cargo test -q -p copilot-contracts export_bindings
	cargo test -q -p copilot-contracts --test fixtures

test:
	cargo test --workspace
	pnpm test

test-db:
	$(WITH_API_ENV) bash scripts/test-db.sh

test-bruno:
	bash scripts/test-bruno.sh

smoke:
	bash scripts/smoke.sh

lint:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets --all-features -- -D warnings
	bash scripts/lint-rust-imports.sh
	pnpm check:all
	pnpm check:markdown

format:
	cargo fmt --all
	pnpm check:fix:all
	pnpm check:markdown:fix

type-check:
	cargo check --workspace --all-targets --all-features
	pnpm type-check

boundaries:
	pnpm boundaries

check: format lint type-check test boundaries

ci: lint type-check test boundaries contracts
	git diff --exit-code -- $(CONTRACTS_TS) $(CONTRACTS_FIXTURES) || (echo "Run 'make contracts' and commit the result." && false)
	@test -z "$$(git ls-files --others --exclude-standard -- $(CONTRACTS_TS) $(CONTRACTS_FIXTURES))" || (echo "Untracked bindings or fixtures: run 'make contracts' and commit." && false)

build:
	cargo build --release -p copilot-api -p copilot-database
	pnpm --filter @copilot/web build

# The Tauri shell (apps/native) builds outside the Cargo workspace and outside `make check`.
NATIVE_CRATE := apps/native/src-tauri/Cargo.toml

native-dev:
	pnpm --filter @copilot/native tauri dev

native-lint:
	cargo fmt --manifest-path $(NATIVE_CRATE) --check
	cargo clippy --manifest-path $(NATIVE_CRATE) --all-targets -- -D warnings

native-test:
	cargo test --manifest-path $(NATIVE_CRATE)

native-build:
	pnpm --filter @copilot/native tauri build --bundles deb

android-dev:
	pnpm --filter @copilot/native tauri android dev

android-build:
	pnpm --filter @copilot/native tauri android build --apk --target aarch64

clean:
	docker compose down -v
	cargo clean
	cargo clean --manifest-path $(NATIVE_CRATE)
	find . -name node_modules -type d -prune -exec rm -rf {} +
	find . \( -name dist -o -name out -o -name .run \) -type d -prune -exec rm -rf {} +
