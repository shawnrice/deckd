#!/bin/bash
# Create the shared local code-signing identity, once, for all personal tools.
# Run this by hand: ./scripts/signing-cert.sh
#
# This is deliberately NOT called from the build. Creating a trusted identity
# needs an interactive keychain authorization that cannot be scripted away
# (without handling your login password, which nothing here will do), so it is
# a one-time human step. Everything after it — signing on each build — is
# automated by scripts/sign.sh.
#
# The identity is intentionally generic rather than per-project. Every tool you
# build locally should share it: TCC keys its grants on the signing identity,
# so one identity means one permission prompt per tool, ever.

set -e
cd "$(dirname "$0")/.."

IDENTITY="${SIGN_IDENTITY:-Shawn Local Dev}"
KEYCHAIN="$HOME/Library/Keychains/login.keychain-db"
DAYS=3650

if security find-identity -v -p codesigning | grep -qF "$IDENTITY"; then
    echo "Identity already present and valid: $IDENTITY"
    echo "Nothing to do — use scripts/sign.sh to sign builds."
    exit 0
fi

# A cert can exist while the identity is still invalid: the import succeeded
# but the trust prompt was dismissed or timed out. That is resumable — re-run
# just the trust step against the cert already in the keychain rather than
# making the user delete and start over.
if security find-certificate -c "$IDENTITY" "$KEYCHAIN" >/dev/null 2>&1; then
    echo "Certificate '$IDENTITY' is present but not yet trusted."
    echo "Re-running the trust step — enter your login password when prompted."
    echo
    RESUME="$(mktemp -d)"
    trap 'rm -rf "$RESUME"' EXIT
    security find-certificate -c "$IDENTITY" -p "$KEYCHAIN" > "$RESUME/cert.pem"
    security add-trusted-cert -r trustRoot -p codeSign -k "$KEYCHAIN" "$RESUME/cert.pem"

    if security find-identity -v -p codesigning | grep -qF "$IDENTITY"; then
        echo
        echo "Identity is now valid: $IDENTITY"
        echo "Sign builds with: ./scripts/sign.sh"
        exit 0
    fi
    echo
    echo "Still not valid — the trust step did not complete."
    echo "To start over: security delete-certificate -c \"$IDENTITY\" \"$KEYCHAIN\""
    exit 1
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# Config-file form rather than -addext: the system openssl is LibreSSL, which
# does not support -addext, and this works identically under both.
cat > "$TMP/req.cnf" <<EOF
[ req ]
distinguished_name = dn
prompt             = no
[ dn ]
CN = $IDENTITY
C  = US
[ codesign ]
basicConstraints = critical,CA:false
extendedKeyUsage = critical,codeSigning
keyUsage         = critical,digitalSignature
EOF

echo "Generating a $DAYS-day self-signed code-signing certificate..."
/usr/bin/openssl req -x509 -newkey rsa:2048 -nodes \
    -keyout "$TMP/key.pem" -out "$TMP/cert.pem" \
    -days "$DAYS" -config "$TMP/req.cnf" -extensions codesign 2>/dev/null

# The p12 password is a throwaway used only to move the key into the keychain;
# the file is deleted on exit and the password is never stored anywhere.
P12PASS="$(/usr/bin/openssl rand -hex 16)"
/usr/bin/openssl pkcs12 -export \
    -inkey "$TMP/key.pem" -in "$TMP/cert.pem" -out "$TMP/bundle.p12" \
    -passout "pass:$P12PASS" -name "$IDENTITY" 2>/dev/null

echo "Importing into the login keychain..."
security import "$TMP/bundle.p12" -k "$KEYCHAIN" -P "$P12PASS" \
    -T /usr/bin/codesign >/dev/null

echo
echo "=> macOS will now ask you to authorize trusting this certificate."
echo "   Enter your login password when prompted. Without this step the"
echo "   identity stays invalid and codesign silently falls back to ad-hoc."
echo
security add-trusted-cert -r trustRoot -p codeSign -k "$KEYCHAIN" "$TMP/cert.pem"

if security find-identity -v -p codesigning | grep -qF "$IDENTITY"; then
    echo
    echo "Identity created: $IDENTITY"
    echo "Sign builds with: ./scripts/sign.sh"
else
    echo
    echo "Import completed but '$IDENTITY' is still not a valid signing identity."
    echo "The trust step was probably cancelled. Delete and retry:"
    echo "  security delete-certificate -c \"$IDENTITY\" \"$KEYCHAIN\""
    exit 1
fi
