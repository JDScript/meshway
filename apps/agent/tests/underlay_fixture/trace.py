"""Audit immediate pre-bind/connect/listen protection in actual strace output.

The factory performs create/mark/readback/bind/connect/listen synchronously on
one thread before yielding; this checker therefore keys that phase by TID/FD.
Async clone FD send fencing and accept inheritance are checked in Rust + TUN.
"""
import re


def audit(text, *, ordinary_connects=8, ordinary_binds=4):
    descriptors, pending = {}, {}
    protected, plain_connects, plain_binds, listeners = 0, 0, 0, 0
    for line in text.splitlines():
        row = re.match(r'^\s*(\d+)\s+(.*)$', line)
        if not row:
            continue
        tid, call = row.groups()
        if '<unfinished ...>' in call:
            pending[tid] = call.split('<unfinished ...>')[0]
            continue
        resumed = re.match(r'<\.\.\. \w+ resumed>(.*)', call)
        if resumed:
            call = pending.pop(tid) + resumed.group(1)
        created = re.match(r'socket\(AF_INET6?, SOCK_(?:STREAM|DGRAM).*\)\s+=\s+(\d+)$', call)
        if created:
            descriptors[(tid, created.group(1))] = {'mark': None, 'verified': False}
            continue
        fd = re.match(r'(\w+)\((\d+)', call)
        if not fd:
            continue
        operation, number = fd.groups()
        key = (tid, number)
        if key not in descriptors:
            continue
        state = descriptors[key]
        marked = re.match(r'setsockopt\(\d+, SOL_SOCKET, SO_MARK, \[(-?\d+)\], 4\)\s+=\s+0$', call)
        readback = re.match(r'getsockopt\(\d+, SOL_SOCKET, SO_MARK, \[(-?\d+)\], \[4\]\)\s+=\s+0$', call)
        if marked:
            state['mark'], state['verified'] = int(marked.group(1)) & 0xffffffff, False
        elif readback:
            state['verified'] = state['mark'] == int(readback.group(1)) & 0xffffffff
        elif operation in ('bind', 'connect', 'listen') and 'AF_NETLINK' not in call:
            is_protected = state['mark'] is not None and state['mark'] & 0xffff0000 == 0x4d570000 and state['verified']
            if is_protected:
                protected += 1
                listeners += operation == 'listen'
            elif operation == 'connect':
                plain_connects += 1
            elif operation == 'bind':
                plain_binds += 1
            else:
                raise AssertionError('unprotected listener: ' + call)
        elif operation == 'close':
            del descriptors[key]
    assert plain_connects == ordinary_connects, (plain_connects, ordinary_connects)
    assert plain_binds == ordinary_binds, (plain_binds, ordinary_binds)
    assert protected > 0, 'no actual protected pre-network syscalls'
    return {'protected_pre_network_calls': protected, 'ordinary_connects': plain_connects,
            'ordinary_binds': plain_binds, 'premarked_listeners': listeners}
