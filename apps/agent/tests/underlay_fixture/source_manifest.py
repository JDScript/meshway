"""One source-input inventory for the namespace fixture image and its callers."""
import hashlib
import json
from pathlib import Path

DIRECTORIES = ('crates', 'apps/controller', 'apps/agent', 'apps/apple/core')
FILES = ('Cargo.toml', 'Cargo.lock', '.dockerignore',
         'experiments/underlay-routing/helper.py', 'experiments/underlay-routing/fixture.py')
EXCLUDED = {'target', 'node_modules', '__pycache__', '.git', '.meshway', '.worktree',
            '.next', '.next-review', '.vitepress', 'dist', 'out', '.build', '.swiftpm'}


def inventory(root):
    root = Path(root)
    inputs = [root / name for name in FILES]
    for name in DIRECTORIES:
        directory = root / name
        if not directory.is_dir():
            raise ValueError('missing fixture input directory: ' + name)
        inputs.extend(path for path in directory.rglob('*')
                      if path.is_file() and not any(part in EXCLUDED for part in path.relative_to(root).parts)
                      and path.name not in ('.DS_Store', '.env')
                      and not path.name.startswith('.env.') and not path.name.endswith('.tsbuildinfo'))
    result = {}
    for path in sorted(inputs):
        if path.is_symlink():
            raise ValueError('fixture inputs must not be symlinks: ' + str(path.relative_to(root)))
        result[path.relative_to(root).as_posix()] = hashlib.sha256(path.read_bytes()).hexdigest()
    return {'format_version': 1, 'files': result}


def verify(actual, expected):
    if not isinstance(actual, dict) or set(actual) != {'format_version', 'files'}:
        raise ValueError('image lacks the complete source manifest; rebuild')
    if type(actual['format_version']) is not int or actual['format_version'] != 1:
        raise ValueError('unsupported image source manifest; rebuild')
    if actual != expected:
        raise ValueError('image build inputs do not match checkout; rebuild')


if __name__ == '__main__':
    import sys
    print(json.dumps(inventory(sys.argv[1]), sort_keys=True))
