"""Schema snapshots of a trusted baseline may not change; new ones may appear."""
import importlib.util
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "ci" / "schema-history.py"
spec = importlib.util.spec_from_file_location("schema_history", SCRIPT)
history = importlib.util.module_from_spec(spec)
spec.loader.exec_module(history)


def run(root, *arguments):
    subprocess.run(["git", *arguments], cwd=root, check=True, capture_output=True)


class SchemaHistoryTests(unittest.TestCase):
    def repository(self, directory):
        root = Path(directory)
        run(root, "init", "-q", "-b", "main")
        run(root, "config", "user.email", "test@example.test")
        run(root, "config", "user.name", "test")
        snapshots = root / history.HISTORY
        snapshots.mkdir(parents=True)
        (snapshots / "v8.sql").write_text("CREATE TABLE a (id INTEGER);\n")
        run(root, "add", "-A")
        run(root, "commit", "-qm", "v8")
        run(root, "tag", "v0.1.0-alpha.1")
        run(root, "checkout", "-qb", "work")
        return root, snapshots

    def test_added_snapshots_pass_and_edited_or_removed_ones_fail(self):
        with tempfile.TemporaryDirectory() as directory:
            root, snapshots = self.repository(directory)
            (snapshots / "v9.sql").write_text("CREATE TABLE b (id INTEGER);\n")
            run(root, "add", "-A")
            run(root, "commit", "-qm", "v9")
            self.assertEqual(history.main(root), 0)
            self.assertEqual(len(history.baselines(root)), 2)

            # Edited together with whatever pins it, in a commit of its own.
            (snapshots / "v8.sql").write_text("CREATE TABLE a (id INTEGER, extra TEXT);\n")
            run(root, "commit", "-qam", "edit v8")
            self.assertEqual(history.violations(root, "v0.1.0-alpha.1"), [f"{history.HISTORY}/v8.sql was modified"])
            self.assertEqual(history.main(root), 1)

            (snapshots / "v8.sql").unlink()
            self.assertEqual(history.violations(root, "main"), [f"{history.HISTORY}/v8.sql was removed"])

    def test_no_baseline_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run(root, "init", "-q", "-b", "other")
            run(root, "config", "user.email", "test@example.test")
            run(root, "config", "user.name", "test")
            (root / "file").write_text("x")
            run(root, "add", "-A")
            run(root, "commit", "-qm", "x")
            self.assertEqual(history.main(root), 1)


if __name__ == "__main__":
    unittest.main()
