#!/bin/bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
tart="${TART:-tart}"
vm=meshway-mac-vm
app="$root/build/DerivedData/Build/Products/${CONFIGURATION:-Debug}/Meshway.app"
test -d "$app"
codesign --verify --deep --strict "$app"
ssh_command=(ssh)
scp_command=(scp -O)
if [[ -n "${MESHWAY_VM_SSH_CONFIG:-}" ]]; then
  ssh_command+=(-F "$MESHWAY_VM_SSH_CONFIG")
  scp_command+=(-F "$MESHWAY_VM_SSH_CONFIG")
  destination="$vm"
else
  ip="$("$tart" ip "$vm")"
  test -n "$ip"
  destination="admin@$ip"
fi
archive="$root/build/Meshway.zip"
ditto -c -k --keepParent "$app" "$archive"
# Resolve the named VM through Tart or its dedicated SSH configuration.
"${scp_command[@]}" "$archive" "$destination:Meshway-upload.zip"
replace="${MESHWAY_VM_REPLACE:-0}"
[[ "$replace" == 0 || "$replace" == 1 ]]
"${ssh_command[@]}" "$destination" "bash -s -- $replace" <<'REMOTE'
set -euo pipefail
test "$(sysctl -n kern.hv_vmm_present)" = 1
staging="$(mktemp -d "$HOME/meshway-stage.XXXXXX")"
trap 'rm -rf "$staging"' EXIT
ditto -x -k "$HOME/Meshway-upload.zip" "$staging"
new="$staging/Meshway.app"
old=/Applications/Meshway.app
codesign --verify --deep --strict "$new"
bundle() { /usr/libexec/PlistBuddy -c 'Print CFBundleIdentifier' "$1/Contents/Info.plist"; }
version() { /usr/libexec/PlistBuddy -c 'Print CFBundleVersion' "$1/Contents/Info.plist"; }
team() { codesign -dv "$1" 2>&1 | sed -n 's/^TeamIdentifier=//p'; }
if [[ -e "$old" ]]; then
  [[ "$1" == 1 ]] || { echo 'Existing App: use MESHWAY_VM_REPLACE=1 for a verified same-identity upgrade' >&2; exit 1; }
  codesign --verify --deep --strict "$old"
  [[ "$(bundle "$new")" == "$(bundle "$old")" && "$(team "$new")" == "$(team "$old")" ]]
  before="$(version "$old")"; after="$(version "$new")"
  [[ "$before" =~ ^[0-9]+$ && "$after" =~ ^[0-9]+$ && ${#before} -le 9 && ${#after} -le 9 ]]
  (( 10#$after > 10#$before )) || { echo 'Replacement requires a higher lab CFBundleVersion' >&2; exit 1; }
  # Only this App executable, never the provider, unrelated apps or Tart.
  pids="$(pgrep -f '^/Applications/Meshway.app/Contents/MacOS/Meshway($| )' || true)"
  if [[ -n "$pids" ]]; then kill -TERM $pids; fi
  backup="$HOME/Meshway-backups/build-$before-$(date -u +%Y%m%dT%H%M%SZ)"
  mkdir -p "$backup"
  mv "$old" "$backup/Meshway.app"
fi
mv "$new" "$old"
codesign --verify --deep --strict "$old"
# The existing extension is replaced by OSSystemExtensionRequest's .replace
# delegate. Keep its approval and every VPN preference; never uninstall first.
"$old/Contents/MacOS/Meshway" --lab activate
REMOTE
