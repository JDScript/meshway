#!/usr/bin/env python3
"""Replay Astra's original counterexamples in a new copy with fixed expectations.

Original snapshot/logs/target remain untouched. Current generation.rs/dns.rs are
copied byte-for-byte. Memory DNS evidence is separate from Linux socket evidence.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[4]
TMP = Path('/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode')
ORIGINAL = TMP / 'meshway-underlay-review-repro'


def checked(text, old, new):
    if text.count(old) != 1:
        raise ValueError('original repro anchor changed: ' + old)
    return text.replace(old, new)


def run(stage):
    directory = Path(tempfile.mkdtemp(prefix='meshway-underlay-review-fixed-', dir=TMP))
    shutil.copytree(ORIGINAL / 'src', directory / 'src')
    for name in ['Cargo.toml', 'Cargo.lock']:
        shutil.copyfile(ORIGINAL / name, directory / name)
    hashes = {}
    for name in ['generation.rs', 'dns.rs']:
        source = ROOT / 'apps/agent/src/underlay' / name
        (directory / 'src/snapshot' / name).write_bytes(source.read_bytes())
        hashes[name] = hashlib.sha256(source.read_bytes()).hexdigest()
    path = directory / 'src/lib.rs'
    text = path.read_text().replace(str(ORIGINAL), str(directory))
    text = checked(text, 'assert!(validate_rules(&reservation(), rules).is_ok());\n            // iproute2',
        'assert!(validate_rules(&reservation(), rules).is_err());\n            // iproute2')
    text = text.replace('BUG: same external capture-table reference rejects numeric ID but accepts unresolved alias',
        'FIXED UR1: numeric ID and unresolved alias both reject outside reserve-table references')
    if stage == 'all':
        text = checked(text, 'assert!(result.is_err());\n        assert_eq!(FIRST_SERVER.load(Ordering::SeqCst), 2);\n        assert_eq!(SECOND_SERVER.load(Ordering::SeqCst), 0);',
            'assert_eq!(result.unwrap().len(), 2);\n        assert_eq!(FIRST_SERVER.load(Ordering::SeqCst), 2);\n        assert_eq!(SECOND_SERVER.load(Ordering::SeqCst), 2);')
        text = text.replace('BUG: SERVFAIL returned after first upstream; healthy second upstream received zero queries',
            'FIXED UR2: SERVFAIL primary and healthy configured backup both received A/AAAA queries')
        # Additional controls run serially: reuse the original wire-building
        # double but vary only service responses, not production resolver code.
        text = checked(text, 'if server.ip().to_string() == "192.0.2.53" {',
            'if server.ip().to_string() == "192.0.2.53" || request.queries[0].name().to_ascii().starts_with("allfail.") {')
        text = checked(text, 'response.metadata.response_code = ResponseCode::ServFail;',
            'response.metadata.response_code = match request.queries[0].name().to_ascii().as_str() {\n'
            ' "missing.test." => ResponseCode::NXDomain, "refused.test." => ResponseCode::Refused, _ => ResponseCode::ServFail };')
        # allfail's second server is counted accurately rather than as primary.
        text = checked(text, 'FIRST_SERVER.fetch_add(1, Ordering::SeqCst);',
            'if server.ip().to_string() == "192.0.2.53" { FIRST_SERVER.fetch_add(1, Ordering::SeqCst); } else { SECOND_SERVER.fetch_add(1, Ordering::SeqCst); }')
        text = checked(text, 'println!("CONTROL: same healthy second upstream alone resolves both A and AAAA");',
            'println!("CONTROL: same healthy second upstream alone resolves both A and AAAA");\n'
            'for (name, backup_count, error_code) in [("allfail.test", 2, "ServFail"), ("missing.test", 0, "NXDomain"), ("refused.test", 0, "Refused")] {\n'
            ' FIRST_SERVER.store(0, Ordering::SeqCst); SECOND_SERVER.store(0, Ordering::SeqCst);\n'
            ' let snapshot = dns::SystemDnsSnapshot::plain_servers(vec!["192.0.2.53:53".parse().unwrap(), "192.0.2.54:53".parse().unwrap()]).unwrap();\n'
            ' let resolver = dns::BootstrapResolver::new(generation::test_factory(), snapshot);\n'
            ' let error = resolver.resolve(Purpose::Controller, name).await.unwrap_err();\n'
            ' assert!(error.to_string().contains(error_code), "{error}");\n'
            ' assert_eq!(FIRST_SERVER.load(Ordering::SeqCst), 2); assert_eq!(SECOND_SERVER.load(Ordering::SeqCst), backup_count);\n'
            ' println!("FIXED UR2 control {name}: {error}, configured backup queries={backup_count}");\n'
            '}')
    path.write_text(text)
    env = {**os.environ, 'CARGO_BUILD_JOBS': '2', 'CARGO_TARGET_DIR': str(ROOT / 'target/underlay-ur-repro')}
    child = subprocess.run(['cargo', 'test', '--locked', '--offline', '--', '--nocapture', '--test-threads=1'],
        cwd=directory, env=env, capture_output=True, text=True, timeout=600)
    (directory / 'tests.stdout').write_text(child.stdout)
    (directory / 'tests.stderr').write_text(child.stderr)
    result = {'stage': stage, 'exit': child.returncode, 'directory': str(directory), 'source_sha256': hashes,
        'original': str(ORIGINAL), 'evidence': 'host control flow, not Linux socket protection'}
    (directory / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(child.stdout)
    print(child.stderr[-4000:])
    print(json.dumps(result, indent=2))
    return child.returncode


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--stage', choices=['ur1', 'all'], default='all')
    raise SystemExit(run(parser.parse_args().stage))
