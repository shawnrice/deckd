# deckd tasks. Use `make install` or `make deploy` rather than a bare
# `cargo install` — cargo leaves an ad-hoc binary and macOS re-prompts for
# Bluetooth/camera permissions on every rebuild. See scripts/sign.sh.

# Every target is a command, not a file, so none of them can be satisfied by
# something on disk with a matching name.
.PHONY: help build install deploy sign signing-cert check logs

.DEFAULT_GOAL := help

help: ## Show available targets
	@grep -hE '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) \
		| awk 'BEGIN{FS=":.*?## "}{printf "  %-13s %s\n", $$1, $$2}'

build: ## Release build
	cargo build --release

install: ## Install and sign (cargo overwrites the binary, taking the signature with it)
	cargo install --path . --force
	./scripts/sign.sh

deploy: install ## Install, sign, and restart the launchd agent
	launchctl kickstart -k gui/$$(id -u)/com.deckd.daemon
	@echo "deckd restarted"

sign: ## Re-sign the installed binary
	./scripts/sign.sh

signing-cert: ## One-time: create the shared local signing identity (interactive)
	./scripts/signing-cert.sh

check: ## cargo check + clippy
	cargo check
	cargo clippy

logs: ## Tail the daemon log
	tail -f /tmp/deckd.stderr.log
