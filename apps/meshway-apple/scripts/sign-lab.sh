#!/bin/bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
test -f "$root/Config/Local.xcconfig"
# Explicit profile-less experiment for the isolated VM. This is not distribution signing.
bash "$root/scripts/build.sh" CODE_SIGNING_ALLOWED=NO
xcodebuild -project "$root/Meshway.xcodeproj" -alltargets \
  -configuration "${CONFIGURATION:-Debug}" -showBuildSettings -json \
  > "$root/build/sign-settings.json"
python3 - "$root" <<'PY'
import json
import pathlib
import plistlib
import subprocess
import sys

root = pathlib.Path(sys.argv[1])
targets = {entry['target']: entry['buildSettings'] for entry in json.loads((root / 'build/sign-settings.json').read_text())}
app = targets['Meshway']
team = app['DEVELOPMENT_TEAM']
identity = app['CODE_SIGN_IDENTITY']
if not team or identity in ('', '-'):
    raise SystemExit('Local.xcconfig must select a Team and a real signing identity')
product = root / 'build/DerivedData/Build/Products' / app['CONFIGURATION'] / 'Meshway.app'
for target, bundle in [('PacketTunnel', product / 'Contents/Library/SystemExtensions' / (app['MESHWAY_EXTENSION_ID'] + '.systemextension')), ('Meshway', product)]:
    settings = targets[target]
    source = root / settings['CODE_SIGN_ENTITLEMENTS']
    text = source.read_text()
    text = text.replace('$(MESHWAY_APP_GROUP)', settings['MESHWAY_APP_GROUP'])
    if '$(' in text:
        raise SystemExit('Unexpanded entitlement build setting')
    entitlements = root / 'build' / (target + '.entitlements')
    entitlements.write_bytes(plistlib.dumps(plistlib.loads(text.encode())))
    subprocess.run(['codesign', '--force', '--sign', identity, '--options', 'runtime', '--timestamp=none', '--entitlements', str(entitlements), str(bundle)], check=True)
subprocess.run(['codesign', '--verify', '--deep', '--strict', str(product)], check=True)
subprocess.run(['codesign', '-d', '-r-', str(product)], check=True)
PY
