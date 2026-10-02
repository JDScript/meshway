#!/usr/bin/env python3
"""Host-only frozen P2a composition. Never alters either task's checkout."""
import argparse
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[4]
TMP = Path('/private/var/folders/8n/20x15sl9797cdg9sdqfj803w0000gn/T/opencode')


def replace(text, old, new):
    if text.count(old) != 1:
        raise ValueError('frozen composition anchor changed: ' + old)
    return text.replace(old, new)


def run(commit):
    directory = Path(tempfile.mkdtemp(prefix='meshway-underlay-p2a-', dir=TMP))
    raw = subprocess.check_output(['git', 'archive', commit, 'Cargo.toml', 'Cargo.lock',
        'crates', 'apps/agent', 'apps/controller', 'apps/apple/core',
        'packages/protocol/openapi.json'], cwd=ROOT)
    with tarfile.open(fileobj=io.BytesIO(raw)) as archive:
        archive.extractall(directory, filter='data')
    destination = directory / 'apps/agent/src/underlay'
    destination.mkdir(parents=True, exist_ok=True)
    for source in (ROOT / 'apps/agent/src/underlay').glob('*.rs'):
        (destination / source.name).write_bytes(source.read_bytes())
    library = directory / 'apps/agent/src/lib.rs'
    library.write_text(replace(library.read_text(), 'pub mod generated;',
        'pub mod generated;\n#[cfg(feature = "managed-connections")]\n#[allow(dead_code)]\nmod underlay;'))
    # Preserve transport-runtime and all P2a manifest choices. Add only the
    # existing dependency lines/feature members owned by the underlay task.
    dependencies = ['bytes', 'http', 'http-body-util', 'hyper', 'hyper-util',
                    'socket2', 'tokio-util', 'rustls-native-certs']
    manifest = directory / 'apps/agent/Cargo.toml'
    text = manifest.read_text()
    current = (ROOT / 'apps/agent/Cargo.toml').read_text().splitlines()
    lines = [next(line for line in current if line.startswith(name + ' = ')) for name in dependencies]
    text = replace(text, '[dependencies]', '[dependencies]\n' + '\n'.join(lines))
    feature = next(line for line in text.splitlines() if line.startswith('managed-connections = '))
    text = replace(text, feature, feature[:-1] + ', ' + ', '.join('"dep:' + name + '"' for name in dependencies) + ']')
    manifest.write_text(text)
    adapter = 'apps/controller/src/bin/generate-rust-client/transport.rs'
    (directory / adapter).write_bytes((ROOT / adapter).read_bytes())
    generator = directory / 'apps/controller/src/bin/generate-rust-client.rs'
    text = generator.read_text()
    text = replace(text, 'mod status_first;',
        'mod status_first;\n#[path = "generate-rust-client/transport.rs"]\nmod transport;')
    text = replace(text, '    status_first::adapt(&mut client.content, operations.len())?;',
        '    status_first::adapt(&mut client.content, operations.len())?;\n    transport::adapt(&mut client.content, operations.len())?;')
    if 'secret_debug::adapt(&mut types.content)?;' not in text:
        raise ValueError('P2a secret_debug adapter was not preserved')
    generator.write_text(text)
    subprocess.run(['git', 'apply', '--unidiff-zero',
                    str(ROOT / 'docs/underlay-integration.patch')], cwd=directory, check=True)
    subprocess.run(['git', 'apply', '--recount', '--unidiff-zero',
                    str(ROOT / 'docs/underlay-p2a.patch')], cwd=directory, check=True)
    env = {**os.environ, 'CARGO_TARGET_DIR': str(ROOT / 'target/underlay-combo')}
    commands = [
        ['cargo', 'run', '-p', 'meshway-controller', '--features', 'codegen', '--bin',
         'generate-rust-client', '--', '--input', 'packages/protocol/openapi.json',
         '--output', 'apps/agent/src/generated'],
        ['cargo', 'fmt', '--all'],
        ['cargo', 'clippy', '--locked', '-p', 'meshway-agent', '--all-features',
         '--all-targets', '--', '-D', 'warnings'],
        ['cargo', 'check', '--locked', '-p', 'meshway-agent', '--no-default-features'],
        ['cargo', 'check', '--locked', '-p', 'meshway-agent', '--no-default-features',
         '--features', 'managed-connections'],
    ]
    result = {'p2a_commit': commit, 'directory': str(directory), 'status': 'failed', 'commands': []}
    try:
        for index, command in enumerate(commands):
            child = subprocess.run(command, cwd=directory, env=env, capture_output=True, text=True, timeout=1200)
            (directory / f'{index}.stdout').write_text(child.stdout)
            (directory / f'{index}.stderr').write_text(child.stderr)
            result['commands'].append({'command': command, 'exit': child.returncode})
            if child.returncode:
                print(child.stderr[-12000:], flush=True)
                raise RuntimeError('composition command failed: ' + ' '.join(command))
        result['status'] = 'ok'
    except BaseException as error:
        result['error'] = str(error)
    (directory / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2), flush=True)
    return 0 if result['status'] == 'ok' else 1


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--p2a', default='56da5ad')
    raise SystemExit(run(parser.parse_args().p2a))
