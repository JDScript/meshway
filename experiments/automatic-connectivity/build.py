"""Bounded non-root Linux compilation from the complete workspace input snapshot."""
import json
import os
import shutil


def build(docker, args, name, logs, source, work, context, manifest, result, owned):
    builder = docker('buildx', 'inspect', 'default').stdout
    assert any(line.split() == ['Driver:', 'docker'] for line in builder.splitlines())
    (logs / 'builder.txt').write_text(builder)
    cid = docker('create', '--name', name + '-compile', '--label', 'meshway.automatic.acceptance=' + name,
        '--user', f'{os.getuid()}:{os.getgid()}', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges',
        '--read-only', '--cpus', '2', '--memory', '4g', '--pids-limit', '512', '--network', 'bridge',
        '--tmpfs', '/tmp:rw,nosuid,nodev,size=256m',
        '--mount', f'type=bind,source={source},target=/source,readonly',
        '--mount', f'type=bind,source={work},target=/work', '--workdir', '/source',
        '--env', 'HOME=/work/home', '--env', 'CARGO_HOME=/work/cargo', '--env', 'CARGO_TARGET_DIR=/work/target',
        '--env', 'CARGO_BUILD_JOBS=2', '--env', 'CARGO_INCREMENTAL=0', '--env', 'RUSTUP_TOOLCHAIN=1.98.0', '--entrypoint', 'cargo', args.compiler,
        'build', '--locked', '-p', 'meshway-agent', '-p', 'meshway-controller', '--all-features').stdout.strip()
    owned.append(cid)
    inspected = json.loads(docker('inspect', cid).stdout)[0]
    (logs / 'compiler-inspect.json').write_text(json.dumps(inspected, indent=2))
    host = inspected['HostConfig']
    assert not host['Privileged'] and host['NetworkMode'] != 'host'
    assert host['CapDrop'] == ['ALL'] and not host.get('CapAdd')
    assert inspected['Config']['User'] == f'{os.getuid()}:{os.getgid()}' and os.getuid() != 0
    compiled = docker('start', '-a', cid, check=False, timeout=1800)
    (logs / 'compile.log').write_text(compiled.stdout + compiled.stderr)
    result['compile_exit'] = json.loads(docker('inspect', cid).stdout)[0]['State']['ExitCode']
    assert result['compile_exit'] == 0, 'Linux build failed'
    for binary in ('meshway-agent', 'meshway-controller'):
        shutil.copy2(work / 'target/debug' / binary, context / binary)
    (context / 'source-manifest.json').write_text(json.dumps(manifest, sort_keys=True))
    (context / 'Dockerfile').write_text(f'FROM {args.runtime}\n'
        'COPY --chmod=755 meshway-agent meshway-controller /usr/local/bin/\n'
        'COPY source-manifest.json /source-manifest.json\n')
    image = 'meshway-automatic-connectivity:' + name
    docker('build', '--builder', 'default', '-t', image, str(context), timeout=600)
    return image
