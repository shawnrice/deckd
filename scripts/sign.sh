#!/bin/bash
# Sign an installed binary with the shared local identity, and prove it took.
# Usage: ./scripts/sign.sh [path-to-binary]     (default: ~/.cargo/bin/deckd)
#
# Why this exists: cargo produces an ad-hoc, linker-signed binary with no
# identity, and TCC (Bluetooth, camera, input monitoring) then keys its grants
# on the cdhash. Every rebuild changes the cdhash, so every rebuild re-prompts
# for permissions. Signing with a stable identity plus a stable identifier
# gives TCC a designated requirement that survives rebuilds.
#
# `cargo install` overwrites the binary and takes the signature with it, so
# this must run after every install, not once.

set -e
cd "$(dirname "$0")/.."

IDENTITY="${SIGN_IDENTITY:-Shawn Local Dev}"
BUNDLE_ID="${SIGN_BUNDLE_ID:-org.shawnrice.deckd}"
TARGET="${1:-$HOME/.cargo/bin/deckd}"

if [ ! -f "$TARGET" ]; then
    echo "error: no such binary: $TARGET" >&2
    exit 1
fi

if ! security find-identity -v -p codesigning | grep -qF "$IDENTITY"; then
    echo "error: no valid signing identity named '$IDENTITY'" >&2
    echo "Create it once with: ./scripts/signing-cert.sh" >&2
    exit 1
fi

# -i pins the identifier. Left alone, the linker invents one with a hash
# suffix (deckd-a538a7f8ede5cc66); the designated requirement includes the
# identifier, so letting it drift would break the TCC match even with a
# stable certificate.
codesign --force --sign "$IDENTITY" --identifier "$BUNDLE_ID" "$TARGET"

# codesign can report success and still leave an ad-hoc signature — that is
# how ~/.cargo/bin/spur ended up unsigned without anyone noticing. Verify the
# result rather than trusting the exit code.
if codesign -dvvv "$TARGET" 2>&1 | grep -q "Signature=adhoc"; then
    echo "error: signature is still ad-hoc — signing did not take" >&2
    echo "The identity exists but is probably untrusted; see scripts/signing-cert.sh" >&2
    exit 1
fi

ACTUAL_ID="$(codesign -dvvv "$TARGET" 2>&1 | sed -n 's/^Identifier=//p')"
if [ "$ACTUAL_ID" != "$BUNDLE_ID" ]; then
    echo "error: identifier is '$ACTUAL_ID', expected '$BUNDLE_ID'" >&2
    exit 1
fi

codesign --verify --strict "$TARGET"

echo "Signed $TARGET"
echo "  identity:   $IDENTITY"
echo "  identifier: $BUNDLE_ID"
echo -n "  requirement: "
codesign -d -r- "$TARGET" 2>&1 | sed -n 's/^designated => //p'
