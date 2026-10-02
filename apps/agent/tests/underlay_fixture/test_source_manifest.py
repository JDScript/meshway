from pathlib import Path
import tempfile
import unittest

from source_manifest import DIRECTORIES, FILES, inventory, verify


class SourceManifestTests(unittest.TestCase):
    def test_nested_modules_libraries_and_input_set_changes_invalidate_image(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for name in DIRECTORIES:
                (root / name).mkdir(parents=True, exist_ok=True)
            for name in FILES:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(name)
            names = ['apps/agent/src/underlay/owner/mod.rs',
                     'apps/agent/src/underlay/owner/bootstrap/mod.rs',
                     'crates/agent-tun/src/lib.rs', 'crates/control-client/src/generated/client.rs']
            for name in names:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(name)
            original = inventory(root)
            self.assertTrue(set(names).issubset(original['files']))
            verify(original, inventory(root))
            for name in names:
                path = root / name
                path.write_text('changed')
                with self.assertRaises(ValueError):
                    verify(original, inventory(root))
                path.write_text(name)
            (root / names[-1]).unlink()
            with self.assertRaises(ValueError):
                verify(original, inventory(root))
            with self.assertRaises(ValueError):
                verify(original['files'], original)
            with self.assertRaises(ValueError):
                verify({**original, 'format_version': True}, original)


if __name__ == '__main__':
    unittest.main()
