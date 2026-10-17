#!/bin/bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
tart="${TART:-tart}"
vm=meshway-mac-vm
app="$root/build/DerivedData/Build/Products/${CONFIGURATION:-Debug}/Meshway.app"
test -d "$app"
codesign --verify --deep --strict "$app"
ip="$("$tart" ip "$vm")"
test -n "$ip"
archive="$root/build/Meshway.zip"
ditto -c -k --keepParent "$app" "$archive"
# The address is always resolved from our named VM. No host installation/activation.
scp "$archive" "admin@$ip:Meshway-upload.zip"
ssh "admin@$ip" 'set -eu; test "$(sysctl -n kern.hv_vmm_present)" = 1; test ! -e /Applications/Meshway.app; ditto -x -k "$HOME/Meshway-upload.zip" /Applications; codesign --verify --deep --strict /Applications/Meshway.app; open /Applications/Meshway.app'
