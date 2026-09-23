# deckd tasks. `just install` is the one to use — a bare `cargo install`
# leaves an ad-hoc binary and macOS re-prompts for Bluetooth/camera
# permissions on every rebuild.

default:
    @just --list

build:
    cargo build --release

# Install and sign (cargo overwrites the binary, taking the signature with it).
install:
    cargo install --path . --force
    ./scripts/sign.sh

# Install, sign, and restart the launchd agent.
deploy: install
    launchctl kickstart -k gui/$(id -u)/com.deckd.daemon
    @echo "deckd restarted"

sign:
    ./scripts/sign.sh

# One-time: create the shared local signing identity (interactive).
signing-cert:
    ./scripts/signing-cert.sh

check:
    cargo check
    cargo clippy

logs:
    tail -f /tmp/deckd.stderr.log
