import unittest
from summarize import describe, kernel_delta, snmp
from run import cpu_details, cpu_window


class EvidenceTests(unittest.TestCase):
    def test_tun_loss_is_attributed_to_its_own_interface_and_direction(self):
        def link(name, dropped):
            return {'ifname':name,'txqlen':32,'mtu':1000,'stats64':{'rx':{'dropped':0},'tx':{'dropped':dropped}}}
        case={'cpu':{'nodes':{'NYC':{}}},'kernel_before':{'NYC':{'links':[link('mwlab0',5),link('eth0',0)]}},
              'kernel_after':{'NYC':{'links':[link('mwlab0',14417),link('eth0',0)]}}}
        links=describe(case)['nodes']['NYC']['links']
        self.assertEqual(links['mwlab0']['tx_dropped'],14412)
        self.assertEqual(links['eth0']['tx_dropped'],0)
        self.assertEqual(links['mwlab0']['rx_dropped'],0)

    def test_snmp_headers_are_matched_and_missing_is_not_zero_loss(self):
        self.assertEqual(snmp('Udp: InDatagrams RcvbufErrors\nUdp: 19 4\n'),
                         {'Udp': {'InDatagrams': 19, 'RcvbufErrors': 4}})
        self.assertIsNone(kernel_delta({}, 'NYC')['Udp.RcvbufErrors'])
        case = {'process_before': {'NYC': {'files': {'net/snmp': 'Udp: RcvbufErrors\nUdp: 4\n'}}},
                'process_after': {'NYC': {'files': {'net/snmp': 'Udp: RcvbufErrors\nUdp: 12\n'}}}}
        self.assertEqual(kernel_delta(case,'NYC')['Udp.RcvbufErrors'],8)

    def test_cpu_window_keeps_user_system_and_explicit_cost_alignment(self):
        def sample(time, usage, user, system):
            return {'NYC': {'monotonic_ns': time, 'files': {'cpu.stat':
                f'usage_usec {usage}\nuser_usec {user}\nsystem_usec {system}\nnr_throttled 0\nthrottled_usec 0'}}}
        window=cpu_window(sample(0,100,60,40),sample(1000000000,500100,300060,200040))
        cpu_details(window,1000000)
        self.assertEqual(window['nodes']['NYC']['cpu_percent_one_core'],50)
        self.assertEqual(window['nodes']['NYC']['user_usec_delta'],300000)
        self.assertEqual(window['nodes']['NYC']['system_usec_delta'],200000)
        self.assertEqual(window['total_cpu_ns_per_receiver_payload_byte'],500)
        self.assertIn('upper estimate',window['alignment'])
        cpu_details(window,0)
        self.assertIsNone(window['total_cpu_ns_per_receiver_payload_byte'])


if __name__=='__main__': unittest.main()
