"""Local refusal/retry tests with a fake command boundary; never invokes Docker/SSH."""
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from restore import restore, OWNER, ATTEMPT


class RestoreBoundary(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name) / 'r1-test-original'
        self.root.mkdir(mode=0o700)
        (self.root / 'owner').write_text('r1-test-original')
        self.data = {'run_id': 'r1-test-original', 'attempt': 'retest-attempt-01', 'action': 'restore-network'}
        self.name = 'meshway-field-r1-test-original-retest-attempt-01'
        self.commands = []
        self.network = None
        self.routes = []

    def tearDown(self):
        self.tmp.cleanup()

    def own_network(self):
        return {'Name': self.name, 'Id': 'a' * 64, 'Labels': {OWNER: self.data['run_id'], ATTEMPT: self.data['attempt']},
                'Driver': 'bridge', 'IPAM': {'Config': [{'Subnet': '172.31.249.0/28', 'Gateway': '172.31.249.1'}]}, 'Containers': {}}

    def fake_run(self, args, **kwargs):
        self.commands.append(args)
        if args[:3] == ['ip', '-j', '-4']:
            output = json.dumps(self.routes)
        elif args[:3] == ['ip', '-j', '-6']:
            output = '[]'
        elif args[1:3] == ['network', 'ls']:
            output = json.dumps({'Name': self.name, 'ID': 'a' * 12}) if self.network else ''
        elif args[1:3] == ['network', 'inspect']:
            output = json.dumps([self.network])
        else:
            raise AssertionError('unexpected mutation or command in refusal/retry test')
        return subprocess.CompletedProcess(args, 0, output.encode(), b'')

    def call(self):
        with patch('restore.subprocess.run', side_effect=self.fake_run):
            return restore(self.data, self.root, ['docker'])

    def test_missing_owner_refuses_before_command(self):
        (self.root / 'owner').unlink()
        with self.assertRaises(ValueError):
            self.call()
        self.assertEqual(self.commands, [])

    def test_unknown_existing_network_is_not_adopted(self):
        self.network = self.own_network()
        self.network['Labels'][ATTEMPT] = 'someone-else'
        with self.assertRaises(ValueError):
            self.call()

    def test_missing_removed_identity_marker_refuses_without_docker(self):
        self.data.update(action='restore-public', role='agent')
        with self.assertRaises(ValueError):
            self.call()
        self.assertEqual(self.commands, [])

    def test_existing_owned_bridge_retry_has_no_create(self):
        self.network = self.own_network()
        self.routes = [{'dst': '172.31.249.0/28', 'dev': 'br-' + 'a' * 12}, {'dst': 'default', 'dev': 'eth0'}]
        self.assertTrue(self.call()['resumed'])

    def test_owned_bridge_does_not_excuse_overlay_route_conflict(self):
        self.network = self.own_network()
        self.routes = [{'dst': '198.18.42.0/24', 'dev': 'existing-user-tunnel'}]
        with self.assertRaises(ValueError):
            self.call()

    def test_cleanup_refuses_attached_container(self):
        self.network = self.own_network()
        self.network['Containers'] = {'unknown-id': {}}
        self.data['action'] = 'restore-cleanup-network'
        with self.assertRaises(ValueError):
            self.call()


if __name__ == '__main__':
    unittest.main()
