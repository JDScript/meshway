"""Local refusal/retry tests with a fake command boundary; never invokes Docker/SSH."""
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from restore import restore, OWNER, ATTEMPT, check_capabilities, check_image_origin, check_runtime_constants


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

    def kernel_routes(self):
        bridge = 'br-' + 'a' * 12
        return [
            {'dst': '172.31.249.0/28', 'dev': bridge, 'protocol': 'kernel', 'scope': 'link', 'prefsrc': '172.31.249.1', 'flags': []},
            {'type': 'local', 'dst': '172.31.249.1', 'dev': bridge, 'table': 'local', 'protocol': 'kernel', 'scope': 'host', 'prefsrc': '172.31.249.1', 'flags': []},
            {'type': 'broadcast', 'dst': '172.31.249.15', 'dev': bridge, 'table': 'local', 'protocol': 'kernel', 'scope': 'link', 'prefsrc': '172.31.249.1', 'flags': []},
        ]

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
        self.routes = self.kernel_routes() + [{'dst': 'default', 'dev': 'eth0'}]
        self.assertTrue(self.call()['resumed'])

    def test_own_bridge_foreign_alias_or_changed_route_attributes_refuse(self):
        self.network = self.own_network()
        self.routes = self.kernel_routes() + [{**self.kernel_routes()[1], 'dst': '172.31.249.2'}]
        with self.assertRaises(ValueError):
            self.call()
        changes = [
            (1, {'dst': '172.31.249.2'}), (2, {'dst': '172.31.249.14'}),
            (0, {'dst': '198.18.42.0/24'}), (0, {'dst': '172.31.249.0/29'}),
            (1, {'dev': 'br-' + 'b' * 12}), (2, {'protocol': 'static'}),
            (1, {'scope': 'link'}), (2, {'scope': 'host'}), (1, {'table': 'main'}),
            (0, {'prefsrc': '172.31.249.2'}), (2, {'type': 'unicast'}),
            (0, {'gateway': '172.31.249.2'}), (1, {'flags': ['onlink']}), (1, {'metric': 7}),
        ]
        for index, change in changes:
            with self.subTest(change=change):
                self.routes = self.kernel_routes()
                self.routes[index].update(change)
                with self.assertRaises(ValueError):
                    self.call()

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

    def test_created_network_has_explicit_gateway_before_first_endpoint(self):
        previous = self.fake_run

        def capture(args, **kwargs):
            if args[1:3] == ['network', 'create']:
                self.commands.append(args)
                self.assertEqual(args[args.index('--gateway') + 1], '172.31.249.1')
                return subprocess.CompletedProcess(args, 0, b'a' * 64, b'')
            return previous(args, **kwargs)

        with patch('restore.subprocess.run', side_effect=capture):
            result = restore(self.data, self.root, ['docker'])
        self.assertFalse(result['resumed'])


class ContainerInspectionBoundary(unittest.TestCase):
    def test_docker_capability_spellings_preserve_exact_agent_and_controller_sets(self):
        for caps in [['NET_ADMIN'], ['CAP_NET_ADMIN']]:
            with self.subTest(caps=caps):
                check_capabilities(caps, ['ALL'], 'agent')
        for caps in [None, []]:
            check_capabilities(caps, ['ALL'], 'controller')

    def test_extra_duplicate_unknown_caps_and_changed_drop_refuse(self):
        for caps in [['CAP_DAC_OVERRIDE'], ['CAP_NET_ADMIN', 'CAP_DAC_OVERRIDE'], ['NET_ADMIN', 'CAP_NET_ADMIN'], ['CAP_NET_ADMIN', 'CAP_NET_ADMIN'], ['CAP_CAP_NET_ADMIN'], []]:
            with self.subTest(caps=caps), self.assertRaises(ValueError):
                check_capabilities(caps, ['ALL'], 'agent')
        for drop in [None, [], ['CAP_ALL'], ['ALL', 'ALL'], ['ALL', 'NET_ADMIN']]:
            with self.subTest(drop=drop), self.assertRaises(ValueError):
                check_capabilities(['CAP_NET_ADMIN'], drop, 'agent')
        with self.assertRaises(ValueError):
            check_capabilities(['CAP_NET_ADMIN'], ['ALL'], 'controller')

    def test_immutable_image_revision_must_match_inventory(self):
        image, source = 'sha256:' + 'a' * 64, 'b' * 40
        check_image_origin({'Id': image, 'Config': {'Labels': {'org.opencontainers.image.revision': source}}}, image, source)
        for labels in [None, {}, {'org.opencontainers.image.revision': 'c' * 40}]:
            with self.subTest(labels=labels), self.assertRaises(ValueError):
                check_image_origin({'Id': image, 'Config': {'Labels': labels}}, image, source)
        with self.assertRaises(ValueError):
            check_image_origin({'Id': 'sha256:' + 'd' * 64, 'Config': {'Labels': {'org.opencontainers.image.revision': source}}}, image, source)

    def runtime(self, role):
        config = {'Env': ['PATH=/usr/bin', 'SSL_CERT_FILE=/private/ca.pem', 'SSL_CERT_DIR=/private/empty-certs']}
        hc = {'Sysctls': {'net.ipv4.ip_forward': '0', 'net.ipv6.conf.all.forwarding': '0', 'net.ipv4.ping_group_range': '0 2147483647'},
              'PidsLimit': 128, 'Memory': 402653184, 'NanoCpus': 1000000000, 'Init': True}
        if role == 'agent':
            hc['Sysctls']['net.ipv6.conf.default.disable_ipv6'] = '1'
        return config, hc

    def test_declared_runtime_constants_both_roles_and_unrelated_env(self):
        for role in ['agent', 'controller']:
            config, hc = self.runtime(role)
            config['Env'].append('IMAGE_DEFAULT=unchanged')
            check_runtime_constants(config, hc, role)

    def test_ssl_env_duplicates_missing_and_changed_values_refuse(self):
        for change in ['duplicate', 'missing', 'wrong-file', 'wrong-directory']:
            config, hc = self.runtime('agent')
            if change == 'duplicate':
                config['Env'].append('SSL_CERT_FILE=/private/ca.pem')
            elif change == 'missing':
                config['Env'].remove('SSL_CERT_DIR=/private/empty-certs')
            elif change == 'wrong-file':
                config['Env'][1] = 'SSL_CERT_FILE=/another/ca.pem'
            else:
                config['Env'][2] = 'SSL_CERT_DIR=/etc/ssl/certs'
            with self.subTest(change=change), self.assertRaises(ValueError):
                check_runtime_constants(config, hc, 'agent')

    def test_resource_and_sysctl_changes_refuse(self):
        for key, value in [('PidsLimit', 129), ('Memory', 536870912), ('NanoCpus', 2000000000), ('Init', False)]:
            config, hc = self.runtime('controller')
            hc[key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                check_runtime_constants(config, hc, 'controller')
        for key in ['net.ipv4.ip_forward', 'net.ipv6.conf.all.forwarding', 'net.ipv4.ping_group_range', 'net.ipv6.conf.default.disable_ipv6']:
            config, hc = self.runtime('agent')
            hc['Sysctls'][key] = 'changed'
            with self.subTest(key=key), self.assertRaises(ValueError):
                check_runtime_constants(config, hc, 'agent')


if __name__ == '__main__':
    unittest.main()
