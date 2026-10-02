"""Fixed public whitelist; usable in an offline uid-0, cap-drop-ALL container."""
import hashlib
import json
import os
import re
from pathlib import Path


def public_state(role):
    root = Path('/private')
    if root.is_symlink() or root.stat().st_uid != 0 or root.stat().st_mode & 0o077:
        raise ValueError('role directory is not private uid 0 state')

    def fixed(name):
        p = root / name
        if p.is_symlink() or not p.is_file() or p.stat().st_size > 1024 * 1024:
            raise ValueError('missing or invalid fixed state file')
        return p.read_bytes()

    seed_bytes = fixed('field.json')
    seed = json.loads(seed_bytes)
    allowed = {'role', 'name', 'controller', 'mesh_prefix', 'mesh_ipv6_prefix'}
    if set(seed) != allowed or seed['role'] != role or not all(isinstance(v, str) for v in seed.values()):
        raise ValueError('unrecognized seed')
    hashes = {name: hashlib.sha256(fixed(name)).hexdigest() for name in ['field.json', 'container.py', 'ca.pem']}
    result = {'seed': seed, 'hashes': hashes}
    if role == 'agent':
        state_bytes = fixed('agent.json')
        if (root / 'agent.json').stat().st_mode & 0o077 or (root / 'agent.json.connection-key.json').stat().st_mode & 0o077:
            raise ValueError('identity/key file is not private')
        state = json.loads(state_bytes)
        key_bytes = fixed('agent.json.connection-key.json')
        key = json.loads(key_bytes)
        http = json.loads(fixed('http.json'))
        if set(http) != {'address', 'body'} or not all(isinstance(v, str) for v in http.values()):
            raise ValueError('unrecognized public HTTP seed')
        if state.get('controller') != seed['controller'] or state.get('name') != seed['name']:
            raise ValueError('identity seed mismatch')
        if key.get('controller') != seed['controller'] or key.get('node_id') != state.get('node_id'):
            raise ValueError('connection key binding mismatch')
        if not (root / 'run.enabled').is_file() or (root / 'run.enabled').is_symlink():
            raise ValueError('original run marker missing')
        materials = root / 'agent.json.transport-materials.json'
        if materials.is_symlink() or not materials.is_file() or materials.stat().st_size == 0 or materials.stat().st_mode & 0o077:
            raise ValueError('original private transport materials missing')
        node_id, public_key = state.get('node_id'), key.get('public_key')
        if not isinstance(node_id, str) or not isinstance(public_key, str) or not re.fullmatch(r'[0-9a-f]{64}', node_id) or not re.fullmatch(r'[0-9a-f]{64}', public_key):
            raise ValueError('missing public identity')
        hashes['agent.json.connection-key.json'] = hashlib.sha256(key_bytes).hexdigest()
        result.update(node_id=node_id, public_key=public_key, http=http,
                      state_sha256=hashlib.sha256(state_bytes).hexdigest())
    elif role == 'controller':
        database = root / 'controller'
        if database.is_symlink() or not database.is_dir():
            raise ValueError('original controller database missing')
        for name in ['controller.sqlite3', 'admin.token']:
            p = database / name
            if p.is_symlink() or not p.is_file() or p.stat().st_size == 0:
                raise ValueError('original controller database/identity incomplete')
            if p.stat().st_mode & 0o077:
                raise ValueError('controller database/identity is not private')
        for name in ['server.pem', 'server.key']:
            hashes[name] = hashlib.sha256(fixed(name)).hexdigest()
    else:
        raise ValueError('unknown role')
    return result
