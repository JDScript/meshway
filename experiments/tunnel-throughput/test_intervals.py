import unittest
from intervals import receiver_windows


class ReceiverWindows(unittest.TestCase):
    def test_uses_receiver_excludes_omit_and_reports_conflicting_timing(self):
        def interval(start,end,seconds,bytes_,omitted=False):
            return {'sum':{'start':start,'end':end,'seconds':seconds,'bytes':bytes_,'sender':False,'omitted':omitted}}
        receiver={'intervals':[interval(-1,0,1,0,True),interval(0,1,1,100),interval(1,2,1,0),interval(2,3,1,300),interval(3,3.001,2,999)]}
        sender={'end':{'sum_received':{'seconds':3,'bytes':400,'bits_per_second':3200/3}},'intervals':[]}
        result=receiver_windows({'reverse':False,'raw_server':receiver,'raw_client':sender})
        self.assertEqual(result['zero_byte_seconds'],1)
        self.assertAlmostEqual(result['zero_byte_window_fraction'],1/3)
        self.assertEqual(result['inconsistent_interval_indices'],[4])
        self.assertAlmostEqual(result['nonzero_window_equivalent_mbps'],.0016)
        reverse={**receiver,'end':sender['end']}
        self.assertEqual(receiver_windows({'reverse':True,'raw_client':reverse,'raw_server':{}})['zero_byte_seconds'],1)
