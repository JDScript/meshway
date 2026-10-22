#!/usr/bin/env python3
"""Fail when a user-visible App string or issue code lacks en and zh-Hans text.

Reads the compiler's extracted keys (SWIFT_EMIT_LOC_STRINGS) from the last
Debug build and the issue codes declared in App/Issues.swift.
"""
import glob
import json
import re
import sys
from pathlib import Path

root = Path(__file__).resolve().parents[1]
catalog = json.loads((root / "App/Localizable.xcstrings").read_text())["strings"]
pattern = str(root / "build/DerivedData/Build/Intermediates.noindex/Meshway.build/*/Meshway.build/Objects-normal/*/*.stringsdata")
used = set()
for path in glob.glob(pattern):
    for entries in json.loads(Path(path).read_text()).get("tables", {}).values():
        used.update(entry["key"] for entry in entries)
if not used:
    sys.exit("No extracted strings; build the App first")
codes = set(re.findall(r'= "((?:ERROR|WARN)_[A-Z0-9_]+)"', (root / "App/Issues.swift").read_text()))
problems = []
for key in sorted(used | codes):
    units = catalog.get(key, {}).get("localizations", {})
    for language in ("en", "zh-Hans"):
        if not units.get(language, {}).get("stringUnit", {}).get("value"):
            problems.append(f"{language}: {key!r}")
stale = sorted(k for k, v in catalog.items() if k not in used | codes and v.get("extractionState") != "manual")
problems += [f"unused: {key!r}" for key in stale]
if problems:
    sys.exit("Localization incomplete:\n" + "\n".join(problems))
print(f"PASS strings: {len(used)} UI keys and {len(codes)} issue codes have en and zh-Hans text")
