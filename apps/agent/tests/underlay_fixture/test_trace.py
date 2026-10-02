import unittest
from trace import audit


class PreConnectAudit(unittest.TestCase):
    def test_requires_mark_readback_before_connect(self):
        trace = '''12 socket(AF_INET, SOCK_STREAM|SOCK_CLOEXEC, IPPROTO_TCP) = 8
12 getsockopt(8, SOL_SOCKET, SO_MARK, [0], [4]) = 0
12 setsockopt(8, SOL_SOCKET, SO_MARK, [1297547264], 4) = 0
12 getsockopt(8, SOL_SOCKET, SO_MARK, [1297547264], [4]) = 0
12 connect(8, {sa_family=AF_INET, sin_port=htons(41861)}, 16) = -1 EINPROGRESS
12 close(8) = 0'''
        self.assertEqual(audit(trace, ordinary_connects=0, ordinary_binds=0)['protected_pre_network_calls'], 1)
        with self.assertRaises(AssertionError):
            audit(trace.replace('12 getsockopt(8, SOL_SOCKET, SO_MARK, [1297547264], [4]) = 0\n', ''), ordinary_connects=0, ordinary_binds=0)
        with self.assertRaises(AssertionError):
            audit(trace.replace('setsockopt(8, SOL_SOCKET, SO_MARK, [1297547264], 4) = 0', 'setsockopt(8, SOL_SOCKET, SO_MARK, [1297547264], 4) = -1 EPERM'), ordinary_connects=0, ordinary_binds=0)

    def test_handles_interleaved_unfinished_syscalls(self):
        trace = '''12 socket(AF_INET6, SOCK_DGRAM|SOCK_CLOEXEC, IPPROTO_UDP <unfinished ...>
12 <... socket resumed>) = 8
12 setsockopt(8, SOL_SOCKET, SO_MARK, [1297547264], 4) = 0
12 getsockopt(8, SOL_SOCKET, SO_MARK, [1297547264], [4]) = 0
12 bind(8, {sa_family=AF_INET6}, 28) = 0'''
        self.assertEqual(audit(trace, ordinary_connects=0, ordinary_binds=0)['protected_pre_network_calls'], 1)


if __name__ == '__main__':
    unittest.main()
