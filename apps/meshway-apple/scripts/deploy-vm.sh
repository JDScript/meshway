#!/bin/bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
tart="${TART:-tart}"
vm=meshway-mac-vm
app="$root/build/DerivedData/Build/Products/${CONFIGURATION:-Debug}/Meshway.app"
test -d "$app"
codesign --verify --deep --strict "$app"
ssh_command=(ssh)
scp_command=(scp)
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
"${ssh_command[@]}" "$destination" 'set -eu; test "$(sysctl -n kern.hv_vmm_present)" = 1; test ! -e /Applications/Meshway.app; ditto -x -k "$HOME/Meshway-upload.zip" /Applications; codesign --verify --deep --strict /Applications/Meshway.app; open /Applications/Meshway.app'
