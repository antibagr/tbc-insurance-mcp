#!/bin/sh
# Create the integration's local signing identity without changing system trust.
set -eu
umask 077

identity='TBC Insurance MCP Local Signing'
keychain="${HOME:?}/Library/Keychains/login.keychain-db"

if [ "$(uname -s)" != Darwin ] || [ ! -f "$keychain" ]; then
    printf '%s\n' 'The macOS login Keychain is required.' >&2
    exit 1
fi

if /usr/bin/security find-certificate -c "$identity" "$keychain" >/dev/null 2>&1; then
    printf '%s\n' 'The local signing certificate already exists; it was left unchanged.'
    exit 0
else
    result=$?
    if [ "$result" -ne 44 ]; then
        printf '%s\n' 'The signing certificate lookup failed; no identity was created.' >&2
        exit 1
    fi
fi

signing_directory=$(mktemp -d /tmp/tbc-local-signing.XXXXXXXX)
case "$signing_directory" in
    /tmp/tbc-local-signing.*) ;;
    *) exit 1 ;;
esac
cleanup() {
    rm -f -- "$signing_directory/key.pem" "$signing_directory/certificate.pem"
    rmdir -- "$signing_directory"
}
trap cleanup EXIT HUP INT TERM

/usr/bin/openssl req -new -x509 -newkey rsa:3072 -sha256 -days 3650 -nodes -batch \
    -subj "/CN=$identity/" \
    -addext 'basicConstraints=critical,CA:FALSE' \
    -addext 'keyUsage=critical,digitalSignature' \
    -addext 'extendedKeyUsage=critical,codeSigning' \
    -keyout "$signing_directory/key.pem" -out "$signing_directory/certificate.pem"

# Only Apple's signer may use this non-extractable key without another approval.
/usr/bin/security import "$signing_directory/key.pem" -k "$keychain" \
    -t priv -x -T /usr/bin/codesign
/usr/bin/security import "$signing_directory/certificate.pem" -k "$keychain" -t cert
printf '%s\n' 'Local signing identity created in login Keychain. System trust is unchanged.'
