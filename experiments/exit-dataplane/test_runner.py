"""Host-only acceptance control tests: no Docker calls or service mutations."""
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location('exit_acceptance_runner', HERE / 'run.py')
runner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runner)


class RunnerTest(unittest.TestCase):
    def test_docker_is_refused_before_any_subprocess_without_shared_descriptor(self):
        with patch.object(runner, 'LOCK_DESCRIPTOR', None), patch.object(runner.subprocess, 'run') as child:
            with self.assertRaisesRegex(RuntimeError, 'shared Meshway performance lock'):
                runner.docker('info')
            child.assert_not_called()

    def test_inherited_launcher_help_does_not_reacquire_or_release_parent_lock(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / 'private.lock'
            env = {**os.environ, 'MESHWAY_PERF_LOCK': str(path)}
            env.pop('MESHWAY_PERF_LOCK_FD', None)
            env.pop('MESHWAY_EXIT_PERF_LOCK_FD', None)
            with patch.dict(os.environ, env, clear=True):
                with runner.performance_lock() as descriptor:
                    child = subprocess.run([sys.executable, str(HERE / 'launch.py'), '--help'],
                                           env={**env, 'MESHWAY_PERF_LOCK_FD': str(descriptor)},
                                           pass_fds=(descriptor,), capture_output=True, text=True, timeout=10)
                    self.assertEqual(child.returncode, 0, child.stderr)
                    self.assertIn('--artifacts', child.stdout)
                    with path.open('r+') as other:
                        with self.assertRaises(BlockingIOError):
                            fcntl.flock(other.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
                    with patch.object(runner, 'LOCK_DESCRIPTOR', descriptor):
                        options = runner.docker_process_options()
                    self.assertEqual(options['pass_fds'], (descriptor,))
                    self.assertEqual(options['env']['MESHWAY_PERF_LOCK_FD'], str(descriptor))

    def test_cleanup_never_deletes_foreign_resources_and_retains_failures(self):
        result = {'run_id': 'meshway-exit-test', 'cases': []}
        fixture = runner.Fixture(SimpleNamespace(image='unused'), result, 'ownership')
        fixture.owned_nodes = ['ours', 'foreign', 'missing']
        fixture.owned_nets = ['our-network']
        removed = []

        def fake(kind, action, *args, **_):
            name = args[-1]
            if action == 'inspect':
                if name == 'missing':
                    return SimpleNamespace(returncode=1, stdout='', stderr='absent')
                labels = {'meshway.exit.owner': 'other-owner' if name == 'foreign' else result['run_id']}
                resource = {'Config': {'Labels': labels}} if kind == 'container' else {'Labels': labels}
                return SimpleNamespace(returncode=0, stdout=json.dumps([resource]), stderr='')
            self.assertEqual(action, 'rm')
            removed.append(name)
            return SimpleNamespace(returncode=int(name == 'our-network'), stdout='', stderr='network busy')

        with patch.object(runner, 'docker', side_effect=fake):
            errors = fixture.cleanup()
        self.assertEqual(removed, ['ours', 'our-network'])
        self.assertEqual(len(errors), 2)
        self.assertTrue(any('ownership mismatch foreign' in e for e in errors))
        self.assertTrue(any('network busy' in e for e in errors))


if __name__ == '__main__':
    unittest.main()
