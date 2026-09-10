#!/bin/sh
# Keep the approved Keychain owner unchanged during ordinary MCP updates.
set -eu
umask 077

update_vault=false
case "$#:$*" in
    0:) ;;
    1:--update-vault) update_vault=true ;;
    *) printf '%s\n' 'Usage: install-macos.sh [--update-vault]' >&2; exit 1 ;;
esac

identity='TBC Insurance MCP Local Signing'
identifier='dev.antibagr.tbc-insurance-mcp'
vault_identifier='dev.antibagr.tbc-insurance-mcp.session-vault'
project_directory=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
binary="${HOME:?}/.cargo/bin/tbc-insurance-mcp"
vault_directory="$HOME/Library/Application Support/tbc-insurance"
vault_binary="$vault_directory/tbc-session-vault"
agent_directory="$HOME/Library/LaunchAgents"
agent_plist="$agent_directory/$vault_identifier.plist"
launch_domain="gui/$(id -u)"

if [ "$(uname -s)" != Darwin ]; then
    printf '%s\n' 'This installer requires macOS.' >&2
    exit 1
fi

# The certificate fingerprint is public metadata, embedded in both signed peers.
fingerprint=$(/usr/bin/security find-certificate -c "$identity" -Z |
    awk '/^SHA-1 hash: / { print tolower($3) }')
case "$fingerprint" in
    ''|*[!0-9a-f]*) printf '%s\n' 'The dedicated signing certificate is unavailable.' >&2; exit 1 ;;
esac
if [ "${#fingerprint}" -ne 40 ]; then
    printf '%s\n' 'The dedicated signing certificate is ambiguous.' >&2
    exit 1
fi
vault_requirement="=identifier \"$vault_identifier\" and certificate leaf = H\"$fingerprint\""
main_requirement="=identifier \"$identifier\" and certificate leaf = H\"$fingerprint\""

if [ -L "$vault_directory" ] || [ -L "$vault_binary" ] || [ -L "$agent_plist" ]; then
    printf '%s\n' 'The credential-service installation contains an unexpected symbolic link.' >&2
    exit 1
fi
mkdir -p "$vault_directory" "$agent_directory"
if [ "$(stat -f '%u:%Lp' "$vault_directory")" != "$(id -u):700" ]; then
    printf '%s\n' 'The credential-service directory must be owned by you with private permissions.' >&2
    exit 1
fi

staged_binary=$(mktemp "$HOME/.cargo/bin/.tbc-insurance-mcp.XXXXXXXX")
staged_vault=$(mktemp "$vault_directory/.vault.XXXXXXXX")
staged_plist=$(mktemp "$agent_directory/.tbc-session-vault.XXXXXXXX")
cleanup() {
    rm -f -- "$staged_binary" "$staged_vault" "$staged_plist"
}
trap cleanup EXIT HUP INT TERM

replace_vault=false
if [ ! -e "$vault_binary" ] || [ "$update_vault" = true ]; then
    TBC_SIGNING_CERT_SHA1="$fingerprint" cargo build --locked --release \
        --manifest-path "$project_directory/crates/session-vault/Cargo.toml" \
        --bin tbc-session-vault
    vault_candidate="$project_directory/crates/session-vault/target/release/tbc-session-vault"
    /usr/bin/codesign --force --sign "$identity" --identifier "$vault_identifier" \
        --options runtime "$vault_candidate"
    /usr/bin/codesign --verify --strict -R "$vault_requirement" "$vault_candidate"
    install -m 700 "$vault_candidate" "$staged_vault"
    /usr/bin/codesign --verify --strict -R "$vault_requirement" "$staged_vault"
    replace_vault=true
else
    /usr/bin/codesign --verify --strict -R "$vault_requirement" "$vault_binary"
    printf '%s\n' 'Existing credential service preserved byte for byte.'
fi

TBC_SIGNING_CERT_SHA1="$fingerprint" cargo build --locked --release \
    --manifest-path "$project_directory/Cargo.toml" --bin tbc-insurance-mcp
candidate="$project_directory/target/release/tbc-insurance-mcp"
/usr/bin/codesign --force --sign "$identity" --identifier "$identifier" \
    --options runtime "$candidate"
/usr/bin/codesign --verify --strict -R "$main_requirement" "$candidate"
install -m 755 "$candidate" "$staged_binary"
/usr/bin/codesign --verify --strict -R "$main_requirement" "$staged_binary"

install -m 600 "$project_directory/scripts/session-vault.plist" "$staged_plist"
/usr/bin/plutil -replace ProgramArguments -json '[]' "$staged_plist"
/usr/bin/plutil -insert ProgramArguments.0 -string "$vault_binary" "$staged_plist"
/usr/bin/plutil -lint "$staged_plist"
if [ "$(/usr/bin/plutil -extract ProgramArguments raw -expect array "$staged_plist")" != 1 ] ||
    [ "$(/usr/bin/plutil -extract ProgramArguments.0 raw -expect string "$staged_plist")" != "$vault_binary" ]; then
    printf '%s\n' 'The credential service must launch without extra arguments.' >&2
    exit 1
fi
agent_changed=false
if [ ! -f "$agent_plist" ] || ! cmp -s "$staged_plist" "$agent_plist"; then
    agent_changed=true
fi
if { [ "$replace_vault" = true ] || [ "$agent_changed" = true ]; } &&
    /bin/launchctl print "$launch_domain/$vault_identifier" >/dev/null 2>&1; then
    /bin/launchctl bootout "$launch_domain/$vault_identifier"
fi
if [ "$replace_vault" = true ]; then
    mv -f -- "$staged_vault" "$vault_binary"
fi
if [ "$agent_changed" = true ]; then
    mv -f -- "$staged_plist" "$agent_plist"
fi
if ! /bin/launchctl print "$launch_domain/$vault_identifier" >/dev/null 2>&1; then
    /bin/launchctl bootstrap "$launch_domain" "$agent_plist"
fi

mv -f -- "$staged_binary" "$binary"
/usr/bin/codesign --verify --strict -R "$main_requirement" "$binary"
/usr/bin/codesign --verify --strict -R "$vault_requirement" "$vault_binary"
printf '%s\n' 'Signed MCP and its on-demand credential service are installed.' \
    'Reconnect the MCP host to load the new server.'
